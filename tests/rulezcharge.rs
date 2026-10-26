#![cfg(feature = "m4-wifi")]
//! RulezCharge (inner://rulezcharge) on the M4, against a local stand-in for cpcrulez.fr: |RZ,
//! ESC past its start screen, ESC for Download/search, a search, RETURN on the first result.
//! It sends the search (HTTPGETMEM + COPYBUF), asks for the entry's details, then downloads the
//! file (HTTPGET), which lands in the SD folder under the name the server gives.
//! The replies follow the site's layout (`/m4/?w=aa&s=`, `?i=`, `?d=`).

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use amsrust::csl_runner::CslRunner;
use amsrust::m4::M4Board;
use amsrust::roms::load_rom;

const ID: &str = "42424";
const FILE: &str = "Test_Game_(2026)__BEMU.dsk";

/// The search's answer: `*02*`, then 234-byte records (the first blank): the id, the name in
/// 64 characters, the description padded with backquotes.
fn search_reply() -> Vec<u8> {
    let mut reply = format!("*02*{:234}", "");
    let record = |id: &str, name: &str, description: &str| {
        let head = format!("{id} {name:<64}{description}");
        format!("{head:`<234}")
    };
    reply += &record(ID, &FILE[..FILE.len() - 4], "Gaming related stuff > Extended DSK/40 Cyls");
    reply += &record("42425", "Another_Game", "Demoscene > 40 Cyls");
    for _ in 0..20 {
        reply += &format!("{:234}", "");
    }
    reply.into_bytes()
}

/// An entry's details: an id, the date, the notes, 182 bytes.
fn info_reply() -> Vec<u8> {
    format!("{:<182}", "a1234    2026-10-26Made for bemu's test/ Extended DSK/40 Cyls").into_bytes()
}

fn download() -> Vec<u8> {
    (0..20_000u32).map(|i| (i * 7 + 3) as u8).collect()
}

/// Answers each request as cpcrulez.fr's M4 pages do; the paths asked are kept.
fn serve(listener: TcpListener, asked: Arc<Mutex<Vec<String>>>) {
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request = String::new();
        reader.read_line(&mut request).unwrap();
        let mut line = String::new();
        while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
            line.clear();
        }
        let path = request.split_whitespace().nth(1).unwrap_or_default().to_owned();
        asked.lock().unwrap().push(path.clone());
        let (headers, body) = if path.starts_with("/m4/?w=") {
            (String::new(), search_reply())
        }
        else if path.starts_with("/m4/?i=") {
            (String::new(), info_reply())
        }
        else if path.starts_with("/m4/?d=") {
            (format!("Content-Disposition: attachment; filename=\"{FILE}\"\r\n"), download())
        }
        else {
            (String::new(), b"?".to_vec())
        };
        let head = format!("HTTP/1.1 200 OK\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n", body.len());
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(&body);
    }
}

#[test]
fn rulezcharge_searches_and_downloads() {
    let dir = std::env::temp_dir().join(format!("bemu-rulezcharge-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let sd = dir.join("sd");
    std::fs::create_dir_all(sd.join("m4")).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let asked = Arc::new(Mutex::new(Vec::new()));
    std::thread::spawn({
        let asked = asked.clone();
        move || serve(listener, asked)
    });

    let mut runner = CslRunner::new(&dir);
    let motherboard = runner.motherboard_mut();
    let board = M4Board::with_bundled_rom(&sd).unwrap().with_host("cpcrulez.fr", &address);
    motherboard.plug_expansion(Box::new(board));
    motherboard.set_upper_rom(5, Some(&load_rom("inner://rulezcharge").unwrap())).unwrap();
    let script = "csl_version 1.0\n\
        wait 4000000\n\
        key_output '|rz\\(RET)'\n\
        wait 16000000\n\
        key_output '\\(ESC)'\n\
        wait 6000000\n\
        key_output '\\(ESC)'\n\
        wait 4000000\n\
        key_output 'test\\(RET)'\n\
        wait 12000000\n\
        key_output '\\(RET)'\n\
        wait 16000000\n";
    runner.run_text(script, &dir).unwrap();

    let asked = asked.lock().unwrap().clone();
    assert!(asked.iter().any(|path| path == "/m4/?w=aa&s=test"), "the search: {asked:?}");
    assert!(asked.iter().any(|path| path == &format!("/m4/?i={ID}")), "the details: {asked:?}");
    assert!(asked.iter().any(|path| path == &format!("/m4/?d=001{ID}")), "the download: {asked:?}");
    let downloaded = std::fs::read(sd.join(FILE)).unwrap_or_else(|e| panic!("{FILE} on the SD card: {e}"));
    assert_eq!(downloaded, download());
    let _ = std::fs::remove_dir_all(&dir);
}
