//! The Assistant's network calls (View ▸ Assistant): to the AI the user chose, with their key.
//!
//! The answer is handed over line by line as it arrives, so a streamed answer shows while it is
//! being written. Error statuses are read like any answer: the provider says what went wrong.

use std::io::{BufRead, BufReader, Read};
use std::time::Duration;

use pdfcraft_ui_egui::assistant::HttpCall;

/// Longest line read (a streamed event is a few hundred bytes; a model list, a few kilobytes).
const MAX_LINE: usize = 8 << 20;

pub fn call(c: &HttpCall, line: &mut dyn FnMut(u16, &str) -> bool) -> Result<u16, String> {
    exchange(&agent(Some(crate::updates::os_roots()?)), c, line)
}

/// An agent that reads error statuses as answers and gives a long answer time to stream.
fn agent(roots: Option<ureq::tls::RootCerts>) -> ureq::Agent {
    let mut config = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(600)))
        .http_status_as_error(false);
    if let Some(roots) = roots {
        config = config.tls_config(ureq::tls::TlsConfig::builder().root_certs(roots).build());
    }
    config.build().new_agent()
}

fn exchange(agent: &ureq::Agent, c: &HttpCall, line: &mut dyn FnMut(u16, &str) -> bool) -> Result<u16, String> {
    // Local models (Ollama) answer over plain http on this computer; everything else is https.
    let ua = concat!("PdfCraft/", env!("CARGO_PKG_VERSION"));
    let response = match (c.method, &c.body) {
        ("POST", Some(body)) => {
            let mut r = agent.post(&c.url).header("user-agent", ua);
            for (k, v) in &c.headers {
                r = r.header(k.as_str(), v.as_str());
            }
            r.send(body.as_str())
        }
        _ => {
            let mut r = agent.get(&c.url).header("user-agent", ua);
            for (k, v) in &c.headers {
                r = r.header(k.as_str(), v.as_str());
            }
            r.call()
        }
    }
    .map_err(|e| format!("couldn't reach the AI ({e})"))?;
    let status = response.status().as_u16();
    let mut reader = BufReader::new(response.into_body().into_reader());
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let n = Read::by_ref(&mut reader).take(MAX_LINE as u64).read_until(b'\n', &mut buf).map_err(|e| format!("the answer was cut off ({e})"))?;
        if n == 0 {
            break;
        }
        let text = String::from_utf8_lossy(&buf);
        if !line(status, text.trim_end_matches(['\r', '\n'])) {
            break;
        }
    }
    Ok(status)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    use super::*;

    /// A one-shot local server (as Ollama answers on this computer): it answers `reply` and hands
    /// back the request it got.
    fn serve(reply: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut got = Vec::new();
            let mut buf = [0u8; 4096];
            // Read the headers and the body (Content-Length).
            loop {
                let n = s.read(&mut buf).unwrap();
                got.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&got).to_string();
                if let Some(end) = text.find("\r\n\r\n") {
                    let len = text
                        .lines()
                        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap()))
                        .unwrap_or(0);
                    if got.len() >= end + 4 + len {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            s.write_all(reply.as_bytes()).unwrap();
            String::from_utf8_lossy(&got).to_string()
        });
        (url, handle)
    }

    #[test]
    fn a_streamed_answer_arrives_line_by_line() {
        let (url, server) = serve(
            "HTTP/1.1 200 OK\r\ncontent-type: application/x-ndjson\r\nconnection: close\r\n\r\n{\"message\":{\"content\":\"Ol\"},\"done\":false}\n{\"message\":{\"content\":\"á\"},\"done\":true}\n",
        );
        let call = HttpCall {
            method: "POST",
            url: format!("{url}/api/chat"),
            headers: vec![("content-type".into(), "application/json".into())],
            body: Some("{\"model\":\"m\"}".into()),
        };
        let mut lines = Vec::new();
        let status = call_for_test(&call, &mut |st, l| {
            lines.push((st, l.to_owned()));
            true
        })
        .unwrap();
        assert_eq!(status, 200);
        assert_eq!(lines.len(), 2);
        assert!(lines[1].1.contains("\"á\""));
        let request = server.join().unwrap();
        assert!(request.starts_with("POST /api/chat"), "{request}");
        assert!(request.ends_with("{\"model\":\"m\"}"), "the body goes out: {request}");
    }

    #[test]
    fn an_error_status_is_read_not_raised() {
        let (url, server) = serve(
            "HTTP/1.1 401 Unauthorized\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{\"error\":{\"message\":\"invalid key\"}}\n",
        );
        let call = HttpCall { method: "GET", url: format!("{url}/v1/models"), headers: Vec::new(), body: None };
        let mut body = String::new();
        let status = call_for_test(&call, &mut |st, l| {
            assert_eq!(st, 401);
            body.push_str(l);
            true
        })
        .unwrap();
        assert_eq!(status, 401);
        assert!(body.contains("invalid key"));
        server.join().unwrap();
    }

    /// [`call`] without the system's certificates (a test machine may have none; plain http
    /// doesn't need them).
    fn call_for_test(c: &HttpCall, line: &mut dyn FnMut(u16, &str) -> bool) -> Result<u16, String> {
        exchange(&agent(None), c, line)
    }
}
