// Inspect what the v3 streams actually look like for code.rs

use nexus_compress::codec;
use std::io::{Cursor, Read};
use nexus_compress::format::{NexusHeader, BlockHeader};

fn main() {
    let data = std::fs::read("corpus/code.rs").unwrap();
    let compressed = codec::compress(&data);
    println!("code.rs: {} -> {} (ratio {:.2}x)",
        data.len(), compressed.len(), data.len() as f64 / compressed.len() as f64);

    let mut cur = Cursor::new(&compressed);
    let header = NexusHeader::read(&mut cur).unwrap();
    println!("Header: version={}, blocks={}", header.version, header.block_count);

    for i in 0..header.block_count {
        let bh = BlockHeader::read(&mut cur).unwrap();
        let start = cur.position() as usize;
        let payload_len = bh.compressed_size as usize;
        let payload = &compressed[start..start + payload_len];
        println!("\nBlock {}: type={:?}, uncomp={}, comp={}",
            i, bh.block_type, bh.uncompressed_size, bh.compressed_size);
        if !payload.is_empty() {
            println!("  tag: {} ({})",
                payload[0],
                match payload[0] {
                    1 => "RAW",
                    2 => "v0 RANS",
                    3 => "DUPLICATE",
                    4 => "v2 RANS",
                    5 => "v3 MULTISTREAM",
                    _ => "unknown"
                });

            if payload[0] == 5 {
                let mut off = 1;
                let lit_table_len = u32::from_le_bytes(payload[off..off+4].try_into().unwrap());
                off += 4;
                let lit_stream_len = u32::from_le_bytes(payload[off..off+4].try_into().unwrap());
                off += 4;
                let len_table_len = u32::from_le_bytes(payload[off..off+4].try_into().unwrap());
                off += 4;
                let len_stream_len = u32::from_le_bytes(payload[off..off+4].try_into().unwrap());
                off += 4;
                let dlo_table_len = u32::from_le_bytes(payload[off..off+4].try_into().unwrap());
                off += 4;
                let dlo_stream_len = u32::from_le_bytes(payload[off..off+4].try_into().unwrap());
                off += 4;
                let dhi_table_len = u32::from_le_bytes(payload[off..off+4].try_into().unwrap());
                off += 4;
                let dhi_stream_len = u32::from_le_bytes(payload[off..off+4].try_into().unwrap());
                off += 4;
                let ops_len = u32::from_le_bytes(payload[off..off+4].try_into().unwrap());
                off += 4;

                println!("  lit:    table={:4} stream={:5}", lit_table_len, lit_stream_len);
                println!("  len:    table={:4} stream={:5}", len_table_len, len_stream_len);
                println!("  dist_lo: table={:4} stream={:5}", dlo_table_len, dlo_stream_len);
                println!("  dist_hi: table={:4} stream={:5}", dhi_table_len, dhi_stream_len);
                println!("  ops:    bytes={}", ops_len);
                println!("  total headers + sections: {}",
                    1 + 4*9 + lit_table_len + lit_stream_len
                    + len_table_len + len_stream_len
                    + dlo_table_len + dlo_stream_len
                    + dhi_table_len + dhi_stream_len + ops_len);
            }
        }
        cur.set_position((start + payload_len) as u64);
    }
}