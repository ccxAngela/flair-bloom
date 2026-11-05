use super::core::{Event, Receipt, BATCH_BYTES};
use std::time::Duration;

pub enum Delivery {
    Receipt(Receipt),
    Rejected,
    Retry,
}

pub fn client() -> Option<reqwest::Client> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .connect_timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .ok()
}

pub async fn bootstrap(client: &reqwest::Client, endpoint: &str) -> Option<serde_json::Value> {
    let mut response = client.get(endpoint).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if body.len() + chunk.len() > 16_384 {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).ok()
}

pub async fn deliver(client: &reqwest::Client, endpoint: &str, batch: &[Event]) -> Delivery {
    match client
        .post(endpoint)
        .json(&serde_json::json!({ "items": batch }))
        .send()
        .await
    {
        Ok(mut response) if response.status().is_success() => {
            let mut body = Vec::new();
            loop {
                match response.chunk().await {
                    Ok(Some(chunk)) if body.len() + chunk.len() <= BATCH_BYTES => {
                        body.extend_from_slice(&chunk)
                    }
                    Ok(None) => {
                        return serde_json::from_slice::<Receipt>(&body)
                            .map_or(Delivery::Retry, Delivery::Receipt)
                    }
                    _ => return Delivery::Retry,
                }
            }
        }
        Ok(response) if [400, 413, 422].contains(&response.status().as_u16()) => Delivery::Rejected,
        _ => Delivery::Retry,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    async fn send(status: &str, body: &str) -> Delivery {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!(
            "http://{}/insight-reports/flair-bloom/anonymous",
            listener.local_addr().unwrap()
        );
        let reply = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 2048];
            loop {
                let size = socket.read(&mut buffer).unwrap();
                if size == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..size]);
                if request.ends_with(b"{\"items\":[]}") {
                    break;
                }
            }
            let request = String::from_utf8(request).unwrap();
            assert!(request.starts_with("POST /insight-reports/flair-bloom/anonymous "));
            assert!(!request.to_ascii_lowercase().contains("authorization:"));
            let _ = socket.write_all(reply.as_bytes());
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let result = deliver(&client, &endpoint, &[]).await;
        server.join().unwrap();
        result
    }

    #[tokio::test]
    async fn receipt_keeps_partial_item_statuses() {
        let Delivery::Receipt(receipt) = send("200 OK", r#"{"items":[{"index":0,"status":"accepted"},{"index":1,"status":"retry","code":"unavailable"}]}"#).await else { panic!("应解析逐项回执") };
        assert_eq!(receipt.items.len(), 2);
    }

    #[tokio::test]
    async fn bootstrap_is_a_bounded_bodyless_request_without_credentials() {
        for (status, body, valid) in [
            (
                "200 OK",
                r#"{"services":{"schemaVersion":1}}"#.to_string(),
                true,
            ),
            ("200 OK", " ".repeat(16_385), false),
            ("200 OK", "bad html".to_string(), false),
            ("503 Service Unavailable", "{}".to_string(), false),
            ("302 Found", "{}".to_string(), false),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let endpoint = format!(
                "http://{}/client-bootstrap/flair-bloom",
                listener.local_addr().unwrap()
            );
            let server = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 2048];
                while !request.ends_with(b"\r\n\r\n") {
                    let size = socket.read(&mut buffer).unwrap();
                    if size == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..size]);
                }
                let request = String::from_utf8(request).unwrap();
                assert!(request.starts_with("GET /client-bootstrap/flair-bloom "));
                assert!(!request.to_ascii_lowercase().contains("authorization:"));
                let reply = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(reply.as_bytes());
            });
            assert_eq!(
                bootstrap(&client().unwrap(), &endpoint).await.is_some(),
                valid
            );
            server.join().unwrap();
        }
    }

    #[tokio::test]
    async fn failures_and_invalid_receipts_have_bounded_retry_paths() {
        for status in [
            "404 Not Found",
            "429 Too Many Requests",
            "503 Service Unavailable",
            "500 Internal Server Error",
            "302 Found",
        ] {
            assert!(matches!(send(status, "{}").await, Delivery::Retry));
        }
        for body in ["broken", r#"{"items":[{"index":0,"status":"unknown"}]}"#] {
            assert!(matches!(send("200 OK", body).await, Delivery::Retry));
        }
        assert!(matches!(
            send("200 OK", &" ".repeat(BATCH_BYTES + 1)).await,
            Delivery::Retry
        ));
        for status in [
            "400 Bad Request",
            "413 Payload Too Large",
            "422 Unprocessable Entity",
        ] {
            assert!(matches!(send(status, "{}").await, Delivery::Rejected));
        }
    }

    #[tokio::test]
    async fn timeout_is_retryable() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_millis(50))
            .build()
            .unwrap();
        assert!(matches!(
            deliver(&client, &endpoint, &[]).await,
            Delivery::Retry
        ));
    }
}
