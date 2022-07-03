//! Create swap files and devices in pure Rust.
//!
//! This library will construct this header, copied from the Linux kernel:
//!
//! ```c
//! // Note that this code snippet is licensed GPL-2.0, matching include/linux/swap.h in the Linux Kernel.
//! union swap_header {
//!     struct {
//!         char reserved[PAGE_SIZE - 10];
//!         char magic[10];                 /* SWAP-SPACE or SWAPSPACE2 */
//!     } magic;
//!     struct {
//!         char            bootbits[1024]; /* Space for disklabel etc. */
//!         __u32           version;
//!         __u32           last_page;
//!         __u32           nr_badpages;
//!         unsigned char   sws_uuid[16];
//!         unsigned char   sws_volume[16];
//!         __u32           padding[117];
//!         __u32           badpages[1];
//!     } info;
//! };
//! ```
//!
//! ```rust
//! use std::io::Cursor;
//!
//! use mkswap::SwapWriter;
//!
//! let mut buffer: Cursor<Vec<u8>> = Cursor::new(vec![0; 40 * 1024]);
//! let size = SwapWriter::new()
//!     .label("🔀".into())
//!     .unwrap()
//!     .write(&mut buffer)
//!     .unwrap();
//!
//! ```
//!
//! ### Notes
//!
//! This library will seek around the file, including back to position 0.
//! If this isn't desirable, consider use fscommon::StreamSlice or sending
//! a pull request.

#![deny(missing_docs)]

use std::io::{Seek, SeekFrom, Write};
use uuid::Uuid;

const MAXIMUM_LABEL_BYTES: usize = 16;
const MINIMUM_PAGES: u32 = 10;

/// A general wrapper to merge std::io::Write and std::io::Seek.
pub trait WriteSeek: Write + Seek {}
impl<T: Write + Seek> WriteSeek for T {}

/// A builder to construct a swap space.
///
/// None of these fields are mandatory: they can all be generated.
pub struct SwapWriter {
    uuid: Option<Uuid>,
    label: Option<String>,
    page_size: Option<u64>,
    size: Option<u64>,
}

impl SwapWriter {
    /// Construct a new SwapWriter with all-default Nones
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            uuid: None,
            label: None,
            page_size: None,
            size: None,
        }
    }

    /// Set the label.
    ///
    /// Must be fewer than MAXIMUM_LABEL_BYTES, or an error is returned.
    pub fn label(mut self, label: String) -> Result<Self, Error> {
        if label.len() > MAXIMUM_LABEL_BYTES {
            return Err(Error::LabelTooLong);
        }

        self.label = Some(label);
        Ok(self)
    }

    /// Specify the filesystem's UUID
    pub fn uuid(mut self, uuid: Uuid) -> Self {
        self.uuid = Some(uuid);
        self
    }

    /// Specify the filesystem's page size
    pub fn page_size(mut self, page_size: u64) -> Self {
        self.page_size = Some(page_size);
        self
    }

    /// Write the configured swap space out to a device.
    ///
    /// If no UUID was specified, a random one will be generated.
    ///
    /// If no size was specified, the size will be detected from the provided handle.
    ///
    /// If no page size was specified, the page size of the runtime system will be used.
    pub fn write<T: WriteSeek>(self, mut handle: T) -> Result<u64, Error> {
        let label = self.label.unwrap_or_default();
        if label.len() > MAXIMUM_LABEL_BYTES {
            return Err(Error::LabelTooLong);
        }
        let uuid = self.uuid.unwrap_or_else(Uuid::new_v4);
        let page_size = self.page_size.unwrap_or(
            page_size::get()
                .try_into()
                .map_err(Error::GiganticPageSize)?,
        );
        let total_size_bytes = match self.size {
            Some(size) => size,
            None => detect_size_bytes(&mut handle).map_err(Error::SizeDetection)?,
        };

        let pages: u32 = (total_size_bytes / page_size)
            .try_into()
            .unwrap_or(u32::MAX);
        if pages < MINIMUM_PAGES {
            return Err(Error::TooFewPages(pages));
        }

        handle
            .seek(SeekFrom::Start(1024))
            .map_err(Error::WriteHeader)?;
        handle
            .write(&[0x01, 0x00, 0x00, 0x00])
            .map_err(Error::WriteHeader)?; // version
        handle
            .write(&(pages - 1).to_ne_bytes())
            .map_err(Error::WriteHeader)?; // last page
        handle
            .write(&[0x00, 0x00, 0x00, 0x00])
            .map_err(Error::WriteHeader)?; // number of bad pages

        handle.write(uuid.as_bytes()).map_err(Error::WriteHeader)?; // sws_uuid
        handle.write(label.as_bytes()).map_err(Error::WriteHeader)?; // sws_volume

        handle
            .seek(SeekFrom::Start(page_size - 10))
            .map_err(Error::WriteHeader)?;
        handle.write(b"SWAPSPACE2").map_err(Error::WriteHeader)?; // magic
        handle
            .seek(SeekFrom::Start(0))
            .map_err(Error::WriteHeader)?;

        Ok(total_size_bytes)
    }
}

/// General errors that can occur while configuring and writing a swap space.
#[derive(Debug)]
pub enum Error {
    /// Your page size can't fit in to a u64.
    GiganticPageSize(std::num::TryFromIntError),

    /// An unspecified IO error occured while trying to detect the size of the swap space.
    SizeDetection(std::io::Error),

    /// The specified label is too long: it must be at most MAXIMUM_LABEL_BYTES bytes long.
    LabelTooLong,

    /// The swap area must be at least MINIMUM_PAGES large. The attached u32 is the
    /// number of pages that were attempted.
    TooFewPages(u32),

    /// An error occurred while writing the swap space header to the area.
    WriteHeader(std::io::Error),
}

fn detect_size_bytes<T: WriteSeek>(mut handle: T) -> Result<u64, std::io::Error> {
    handle.seek(SeekFrom::End(0))?;
    let size: u64 = handle.stream_position()?;
    handle.seek(SeekFrom::Start(0))?;

    Ok(size)
}

#[cfg(test)]
mod test {
    use super::*;
    use hex_slice::AsHex;
    use std::io::{Cursor, Read};
    use std::process::Command;
    use tempfile::NamedTempFile;

    #[test]
    fn mkswap_compare() -> Result<(), std::io::Error> {
        let cmdout = NamedTempFile::new()?;
        cmdout.as_file().set_len(40 * 1024)?;

        println!(
            "{:#?}",
            Command::new("mkswap")
                .args(&["--label", "🔀"])
                .args(&["--uuid", "87705c6e-9673-4283-b33a-b87dbf6ec490"])
                .args(&["--pagesize", "4096"])
                .arg(cmdout.path())
                .arg("40")
                .output()
                .unwrap()
        );

        let mut nativeout: Cursor<Vec<u8>> = std::io::Cursor::new(vec![0; 40 * 1024]);
        SwapWriter::new()
            .label("🔀".into())
            .unwrap()
            .uuid(Uuid::parse_str("87705c6e-9673-4283-b33a-b87dbf6ec490").unwrap())
            .page_size(4096)
            .write(&mut nativeout)
            .unwrap();

        let mut cmdbytes: Vec<u8> = vec![];
        cmdout.as_file().read_to_end(&mut cmdbytes).unwrap();

        assert_eq!(
            format!("{:x}", nativeout.into_inner().plain_hex(false)),
            format!("{:x}", cmdbytes.plain_hex(false))
        );

        Ok(())
    }
}
