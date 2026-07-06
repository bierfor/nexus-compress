use nexus_compress::cdc::chunkify;
use nexus_compress::dict_codec::dict_select_for_block;
fn main() {
    for name in &["code.rs", "text.txt", "data.json", "mixed.bin", "trained.dict", "repetitive.bin", "random.bin"] {
        let data = std::fs::read(format!("corpus/{}", name)).unwrap();
        let chunks = chunkify(&data, 4*1024, 64*1024, 15);
        let mut none_n = 0; let mut trained_n = 0; let mut code_n = 0; let mut json_n = 0; let mut local_n = 0;
        for i in 0..chunks.len() {
            let (s, l) = chunks[i];
            let b = &data[s..s+l];
            let sel = dict_select_for_block(b);
            match sel {
                0 => none_n += 1,
                1 => trained_n += 1,
                2 => code_n += 1,
                3 => json_n += 1,
                4 => local_n += 1,
                _ => {}
            }
        }
        println!("{:14}  none={} trained={} code={} json={} local={} (total {})",
            name, none_n, trained_n, code_n, json_n, local_n, chunks.len());
    }
}
