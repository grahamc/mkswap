# mkswap in pure Rust

Initialize a swap device or file in Rust.

For example:

```
```rust
use std::io::Cursor;

use mkswap::SwapWriter;

let mut buffer: Cursor<Vec<u8>> = Cursor::new(vec![0; 40 * 1024]);
let size = SwapWriter::new()
    .label("🔀".into())
    .unwrap()
    .write(&mut buffer)
    .unwrap();

```