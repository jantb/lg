//! The model name shown at startup and in the model picker, when the server
//! that would list the models does not answer.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

/// Asking which model answers is what the UI does to draw the model name, so
/// it must not wait on a server that has not said what it serves; once the
/// server does say, the name is the model it serves.
#[test]
fn the_model_name_is_known_at_once_and_follows_the_server_once_it_answers() {
    let temp = tempfile::tempdir().unwrap();
    // Accepts the connection and never says a word.
    let silent = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", silent.local_addr().unwrap());
    // SAFETY: this is the only test in the binary, and nothing else runs yet.
    unsafe {
        std::env::set_var("LG_PREFERENCES_DIR", temp.path().join("config"));
        std::env::set_var("LG_SETTINGS_DIR", temp.path().join("legacy"));
        std::env::set_var("LG_CONFIG_FILE", temp.path().join("model"));
        std::env::set_var("LG_MTPLX_URL", &url);
        std::env::remove_var("LG_MTPLX_CHAT_ENDPOINT");
        std::env::remove_var("LG_LLM_MODEL");
        std::env::remove_var("LG_LLM_PROVIDER");
    }
    std::env::set_current_dir(temp.path()).unwrap();

    let asked = Instant::now();
    let model = lg::llm::current_model();

    assert!(!model.is_empty());
    assert!(
        asked.elapsed() < Duration::from_millis(500),
        "naming the model waited {:?} on the server",
        asked.elapsed()
    );

    let answering = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", answering.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in answering.incoming().flatten() {
            let mut reader = BufReader::new(&stream);
            let mut line = String::new();
            while reader.read_line(&mut line).is_ok_and(|read| read > 2) {
                line.clear();
            }
            let body = r#"{"data":[{"id":"served-model"}]}"#;
            let _ = write!(
                &stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    // SAFETY: still the only test in the binary.
    unsafe { std::env::set_var("LG_MTPLX_URL", &url) };
    lg::llm::prime_models_async();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !lg::llm::models_arrived() {
        assert!(
            Instant::now() < deadline,
            "the server's answer never landed"
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(lg::llm::current_model(), "served-model");
}
