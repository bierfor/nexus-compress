//! Sprint 5.6.9 — end-to-end test that EXACTLY mirrors what the
//! GUI does when the user pastes a v3 token and clicks "Recibir":
//!
//! 1. Sender: start_direct_sender (no cloudflared, no Quick tunnel).
//! 2. Receiver: peek_filename → fetch /meta without doing the receive.
//! 3. Receiver: receive_direct_file (does the full SPAKE2 + file
//!    transfer).
//! 4. Assert: bytes match the original.

#[path = "../archive_inspect.rs"]
mod archive_inspect;
#[path = "../p2p_auth.rs"]
mod p2p_auth;
#[path = "../p2p_config.rs"]
mod p2p_config;
#[path = "../p2p_tunnel.rs"]
mod p2p_tunnel;
#[path = "../upnp_hole.rs"]
mod upnp_hole;

use p2p_tunnel::{peek_filename, receive_direct_file, start_direct_sender};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::time::timeout;

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    // 1. Setup: create a temp file with known content.
    let tmp = std::env::temp_dir().join(format!("e2e_v3_{}.html", std::process::id()));
    let original = b"<html><body>test content for Sofia Baldi's CV</body></html>";
    {
        let mut f = tokio::fs::File::create(&tmp).await.expect("create");
        f.write_all(original).await.expect("write");
    }

    // 2. Force Direct mode in BOTH possible config locations.
    let cfg_dir_a = std::env::temp_dir().join("nexus-rar-p2p");
    std::fs::create_dir_all(&cfg_dir_a).unwrap();
    std::fs::write(
        cfg_dir_a.join("nexus_config.json"),
        br#"{"mode":"Direct","hostname":null}"#,
    )
    .unwrap();
    // The GUI binary uses tauri::path::app_data_dir → on macOS that's
    // ~/Library/Application Support/com.nexus-rar.tunnel
    let cfg_dir_b = std::env::var("HOME")
        .map(|h| {
            std::path::PathBuf::from(h).join("Library/Application Support/com.nexus-rar.tunnel")
        })
        .unwrap_or_else(|_| std::env::temp_dir().join("nexus-rar-home"));
    let _ = std::fs::create_dir_all(&cfg_dir_b);
    let _ = std::fs::write(
        cfg_dir_b.join("nexus_config.json"),
        br#"{"mode":"Direct","hostname":null}"#,
    );

    // 3. Start the sender on 0.0.0.0:0 — get a free port.
    let code = "alpha-bear-cosmic-delta".to_string();
    let started = start_direct_sender(tmp.clone(), code.clone(), cfg_dir_a.clone())
        .await
        .expect("start sender");

    let v3_token = started.token_compact.clone();
    println!("[e2e] token = {}", v3_token);
    println!(
        "[e2e] token.v = {}, filename = {:?}",
        started.token.v, started.token.filename
    );
    assert!(
        started.token.filename.is_some(),
        "sender must include filename in token"
    );
    assert_eq!(
        started.token.filename.as_deref(),
        Some(tmp.file_name().unwrap().to_str().unwrap()),
        "filename must be the original file's basename"
    );

    // 4. PEEK the filename (the new command). Should succeed without
    //    consuming the SPAKE state.
    let peeked = peek_filename(&v3_token, 5).await.expect("peek_filename");
    println!("[e2e] peeked filename = {:?}", peeked);
    assert_eq!(peeked.as_deref(), started.token.filename.as_deref());

    // 5. PEEK again to confirm the first one didn't burn anything.
    let peeked2 = peek_filename(&v3_token, 5).await;
    println!("[e2e] second peek = {:?}", peeked2);
    assert!(peeked2.is_ok(), "peek should be idempotent");

    // 6. Now do the FULL receive. SPAKE state is fresh.
    let out = std::env::temp_dir().join(format!("e2e_v3_RECEIVED_{}.html", std::process::id()));
    let _ = std::fs::remove_file(&out);

    let result = timeout(
        Duration::from_secs(15),
        receive_direct_file(v3_token.clone(), out.clone(), 5),
    )
    .await
    .expect("receive timed out")
    .expect("receive failed");

    println!(
        "[e2e] received {} bytes → {} (filename={:?})",
        result.bytes_written,
        result.output_path.display(),
        result.filename
    );
    assert_eq!(result.bytes_written as usize, original.len(), "bytes match");
    assert_eq!(
        result.filename.as_deref(),
        started.token.filename.as_deref(),
        "filename flows through receive result"
    );

    // 7. Verify the actual bytes on disk.
    let got = std::fs::read(&out).expect("read received file");
    assert_eq!(got, original, "file bytes match");

    // 8. Double-receive regression test (Sprint 5.6.12).
    //    Send the same token through a SECOND receive call.
    //    Previously this 409'd because handle_spake consumed
    //    the state. Now it should succeed because the
    //    regenerate-on-empty path kicks in.
    println!("[e2e] regression: double-receive with same token...");
    let out2 = std::env::temp_dir().join(format!("e2e_v3_RECEIVED2_{}.html", std::process::id()));
    let _ = std::fs::remove_file(&out2);
    let result2 = timeout(
        Duration::from_secs(15),
        receive_direct_file(v3_token.clone(), out2.clone(), 5),
    )
    .await
    .expect("second receive timed out")
    .expect("second receive failed");
    let got2 = std::fs::read(&out2).expect("read second file");
    assert_eq!(got2, original, "second-receive bytes match");
    assert_eq!(result2.bytes_written as usize, original.len());
    println!("[e2e] double-receive OK — {} bytes", result2.bytes_written);

    // 9. Multi-chunk regression test (Sprint 5.6.13).
    //    Send a file LARGER than the 64 KiB chunk size so the
    //    stream encrypt emits more than one chunk. Previously
    //    the receiver's ChunkCipher::open_chunk forgot to
    //    increment its counter, so every chunk after the
    //    first decrypted with the wrong nonce and failed
    //    AES-GCM auth ("decrypt chunk @65536: p2p: auth...").
    println!("[e2e] regression: multi-chunk file (128 KiB)...");
    let tmp_multi = std::env::temp_dir().join(format!("e2e_v3_multi_{}.bin", std::process::id()));
    let multi: Vec<u8> = (0..(128 * 1024)).map(|i| (i % 251) as u8).collect();
    {
        let mut f = tokio::fs::File::create(&tmp_multi)
            .await
            .expect("create multi");
        f.write_all(&multi).await.expect("write multi");
    }
    let started_multi = start_direct_sender(
        tmp_multi.clone(),
        "alpha-bear-cosmic-delta".to_string(),
        cfg_dir_a.clone(),
    )
    .await
    .expect("start multi sender");
    let multi_token = started_multi.token_compact.clone();
    let out_multi =
        std::env::temp_dir().join(format!("e2e_v3_MULTI_RECEIVED_{}.bin", std::process::id()));
    let _ = std::fs::remove_file(&out_multi);
    let result_multi = timeout(
        Duration::from_secs(15),
        receive_direct_file(multi_token, out_multi.clone(), 5),
    )
    .await
    .expect("multi receive timed out")
    .expect("multi receive failed");
    let got_multi = std::fs::read(&out_multi).expect("read multi");
    assert_eq!(got_multi, multi, "multi-chunk bytes match");
    assert_eq!(result_multi.bytes_written as usize, multi.len());
    println!(
        "[e2e] multi-chunk OK — {} bytes ({} chunks)",
        result_multi.bytes_written,
        (multi.len() + 65535) / 65536
    );
    drop(started_multi);
    let _ = std::fs::remove_file(&tmp_multi);
    let _ = std::fs::remove_file(&out_multi);

    // 10. Folder-send regression (Sprint 5.6.16). Sender tars
    //     the directory using system tar. Receiver gets a .tar
    //     that can be untarred with the standard system tool
    //     (or by double-click in Finder).
    println!("[e2e] regression: folder -> .tar roundtrip...");
    let folder = std::env::temp_dir().join(format!("e2e_v3_folder_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir(&folder).expect("mkdir folder");
    for i in 0..5 {
        std::fs::write(
            folder.join(format!("file_{}.txt", i)),
            format!("content of file {}", i).as_bytes(),
        )
        .expect("write folder file");
    }
    let started_folder = start_direct_sender(
        folder.clone(),
        "alpha-bear-cosmic-delta".to_string(),
        cfg_dir_a.clone(),
    )
    .await
    .expect("start folder sender");
    assert!(
        started_folder.token.filename.as_deref() == Some("folder.tar")
            || started_folder.token.filename.as_deref()
                == Some(&format!("e2e_v3_folder_{}.tar", std::process::id())),
        "filename should be .tar not .nxs6: got {:?}",
        started_folder.token.filename
    );
    let folder_token = started_folder.token_compact.clone();
    let out_folder = std::env::temp_dir().join(format!("e2e_v3_FOLDER_RECEIVED.tar",));
    let _ = std::fs::remove_file(&out_folder);
    let result_folder = timeout(
        Duration::from_secs(15),
        receive_direct_file(folder_token, out_folder.clone(), 5),
    )
    .await
    .expect("folder receive timed out")
    .expect("folder receive failed");
    // Verify the received file is a valid tar archive by
    // listing it with /usr/bin/tar.
    let listing = std::process::Command::new("/usr/bin/tar")
        .arg("-tf")
        .arg(&out_folder)
        .output()
        .expect("tar list");
    assert!(
        listing.status.success(),
        "received file is not a valid tar: stderr={}",
        String::from_utf8_lossy(&listing.stderr)
    );
    let listing_stdout = String::from_utf8_lossy(&listing.stdout);
    for i in 0..5 {
        assert!(
            listing_stdout.contains(&format!("file_{}.txt", i)),
            "tar missing file_{}.txt. Listing:\n{}",
            i,
            listing_stdout
        );
    }
    println!(
        "[e2e] folder OK — received .tar with {} bytes, 5 entries verified",
        result_folder.bytes_written
    );
    drop(started_folder);
    let _ = std::fs::remove_dir_all(&folder);
    // NOTE: out_folder kept on disk for the inspection test
    // below (Sprint 5.6.17). Cleaned up at the end.

    // 11. Archive inspection regression (Sprint 5.6.17). Verify
    //     list_entries reads the tar TOC without extracting, and
    //     extract_entries with a subset only writes the chosen
    //     files.
    println!("[e2e] regression: archive inspection (WinRAR-style browsing)...");
    let entries = crate::archive_inspect::list_entries(&out_folder).expect("list entries");
    // tar archives contain directory entries alongside files,
    // and on some systems long-name entries get split into
    // separate headers. We don't assert exact count — we just
    // check that all 5 file_* entries are listed with the
    // right basename and non-zero size.
    assert!(!entries.is_empty(), "tar should have entries");
    for i in 0..5 {
        let hit = entries
            .iter()
            .any(|e| !e.is_dir && e.name.ends_with(&format!("file_{}.txt", i)) && e.size > 0);
        assert!(hit, "file_{}.txt missing from entries", i);
    }
    let selective_dir =
        std::env::temp_dir().join(format!("e2e_v3_SELECTIVE_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&selective_dir);
    let selected = vec![
        format!("e2e_v3_folder_{}/file_1.txt", std::process::id()),
        format!("e2e_v3_folder_{}/file_3.txt", std::process::id()),
    ];
    let written = crate::archive_inspect::extract_entries(
        &out_folder,
        &selective_dir,
        Some(selected.clone()),
    )
    .expect("selective extract");
    assert_eq!(written.len(), 2, "selective should write 2 files");
    // WinRAR-style: preserve the archive's internal hierarchy.
    // Both extracted files end up under dest/<folder>/.
    let sub_dir = selective_dir.join(format!("e2e_v3_folder_{}", std::process::id()));
    let mut read_dir = std::fs::read_dir(&sub_dir).expect("readdir sub");
    let mut names: Vec<String> = read_dir
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec!["file_1.txt".to_string(), "file_3.txt".to_string()]
    );
    println!(
        "[e2e] inspection OK — listed {} entries, selectively extracted 2",
        entries.len()
    );
    let _ = std::fs::remove_dir_all(&selective_dir);
    let _ = std::fs::remove_file(&out_folder);

    // 12. .nxs6 (V6Solid) inspection regression (Sprint 5.6.20).
    //     Verify list_solid_entries reads the .nxs6 TOC without
    //     LZMA-decompressing the payload, and selective
    //     extraction writes only the chosen entries.
    println!("[e2e] regression: .nxs6 (V6Solid) central directory...");
    let nxs6_src = std::env::temp_dir().join(format!("e2e_v3_nxs6_src_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&nxs6_src);
    std::fs::create_dir(&nxs6_src).expect("mkdir nxs6 src");
    for i in 0..4 {
        std::fs::write(
            nxs6_src.join(format!("binary_{}.dat", i)),
            vec![(i as u8); 64 * 1024], // 64 KiB of deterministic data
        )
        .expect("write nxs6 file");
    }
    let (result, archive_bytes) = nexus_compress::api::compress_directory_with_backend(
        &nxs6_src,
        nexus_compress::api::CompressionBackend::V6Solid,
        1,
    )
    .expect("v6solid compress");
    let nxs6_path = std::env::temp_dir().join(format!("e2e_v3_solid_{}.nxs6", std::process::id()));
    let _ = std::fs::remove_file(&nxs6_path);
    std::fs::write(&nxs6_path, &archive_bytes).expect("write nxs6");
    // List
    let entries_solid = archive_inspect::list_entries(&nxs6_path).expect("list nxs6");
    assert_eq!(
        entries_solid.len(),
        result.entries.len(),
        "list_entries must match the in-memory TOC"
    );
    for e in &entries_solid {
        assert!(e.size > 0);
        assert!(!e.is_dir);
    }
    // Selective extract 2 of the 4
    let selective_solid_dir =
        std::env::temp_dir().join(format!("e2e_v3_SOLID_OUT_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&selective_solid_dir);
    let picks: Vec<String> = entries_solid
        .iter()
        .filter(|e| e.name.ends_with("binary_0.dat") || e.name.ends_with("binary_2.dat"))
        .map(|e| e.name.clone())
        .collect();
    assert_eq!(picks.len(), 2);
    let written =
        archive_inspect::extract_entries(&nxs6_path, &selective_solid_dir, Some(picks.clone()))
            .expect("extract nxs6 selective");
    assert_eq!(written.len(), 2);
    let mut read = std::fs::read_dir(&selective_solid_dir).expect("readdir");
    let mut names: Vec<String> = read
        .map(|x| x.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec!["binary_0.dat".to_string(), "binary_2.dat".to_string()]
    );
    // Verify byte content (raw preprocessor preserves bytes).
    let b0 = std::fs::read(selective_solid_dir.join("binary_0.dat")).expect("read b0");
    assert_eq!(b0, vec![0u8; 64 * 1024]);
    println!(
        "[e2e] .nxs6 inspection OK — listed {} entries, extracted 2 raw-bytes-verified",
        entries_solid.len()
    );
    let _ = std::fs::remove_dir_all(&nxs6_src);
    let _ = std::fs::remove_dir_all(&selective_solid_dir);
    let _ = std::fs::remove_file(&nxs6_path);
    let out2 = std::env::temp_dir().join(format!("e2e_v3_RECEIVED2_{}.html", std::process::id()));
    let _ = std::fs::remove_file(&out2);
    let result2 = timeout(
        Duration::from_secs(15),
        receive_direct_file(v3_token.clone(), out2.clone(), 5),
    )
    .await
    .expect("second receive timed out")
    .expect("second receive failed");
    let got2 = std::fs::read(&out2).expect("read second file");
    assert_eq!(got2, original, "second-receive bytes match");
    assert_eq!(result2.bytes_written as usize, original.len());
    println!("[e2e] double-receive OK — {} bytes", result2.bytes_written);

    // 8. Cleanup.
    drop(started);
    let _ = std::fs::remove_file(&tmp);
    let _ = std::fs::remove_file(&out);
}
