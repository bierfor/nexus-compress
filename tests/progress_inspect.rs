use std::time::Instant;

#[test]
fn inspect_actual_progress_events() {
    let input: Vec<u8> = (0..5_000_000u32).map(|i| (i.wrapping_mul(31) ^ 0xa5) as u8).collect();
    let start = Instant::now();
    let mut events: Vec<(u128, u64)> = Vec::new();
    let _ = nexus_compress::codec::compress_with_progress(&input, |cumulative| {
        let elapsed = start.elapsed().as_millis();
        events.push((elapsed, cumulative));
    });
    println!("=== Progress event inspection ===");
    println!("Input: {} bytes ({:.2} MB)", input.len(), input.len() as f64 / 1024.0 / 1024.0);
    println!("Total events fired: {}", events.len());
    if !events.is_empty() {
        println!("First: elapsed_ms={} cumulative={}", events[0].0, events[0].1);
        let mid = events.len() / 2;
        println!("Mid #{}: elapsed_ms={} cumulative={}", mid, events[mid].0, events[mid].1);
        println!("Last:  elapsed_ms={} cumulative={}", events.last().unwrap().0, events.last().unwrap().1);
    }
    println!("Final elapsed: {} ms", start.elapsed().as_millis());
}
