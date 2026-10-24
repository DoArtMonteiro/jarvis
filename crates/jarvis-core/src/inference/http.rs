//! Just enough HTTP/1.1 to talk to the local model server on loopback.
//!
//! The server is code the Core trusts to run, but what it sends back is
//! derived from model output, so every read here is bounded: the header
//! block, each chunk, each line and the whole body. Only `127.0.0.1` is ever
//! contacted; the port and key come from the Core, which started the server.

use std::net::{Ipv4Addr, SocketAddrV4};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const MAX_HEAD_BYTES: usize = 16 * 1024;
const MAX_LINE_BYTES: usize = 256 * 1024;
/// Largest response body read.
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("model server connection failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("model server sent an invalid response: {0}")]
    Invalid(&'static str),
    #[error("model server answered {status}: {body}")]
    Status { status: u16, body: String },
}

/// One request. `body` is JSON when present.
pub struct Request<'a> {
    pub method: &'static str,
    pub path: &'static str,
    pub key: &'a str,
    pub body: Option<&'a [u8]>,
}

/// Without the key or the body (which holds the conversation).
impl std::fmt::Debug for Request<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request")
            .field("method", &self.method)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

/// An open response whose body is read incrementally.
#[derive(Debug)]
pub struct Response {
    stream: TcpStream,
    pub status: u16,
    chunked: bool,
    /// Bytes read past the header block, not yet decoded.
    buffer: Vec<u8>,
    /// Bytes left in the current chunk.
    chunk_left: usize,
    body_read: usize,
    done: bool,
    /// Decoded body bytes not yet returned as a line.
    pending: Vec<u8>,
}

pub async fn send(port: u16, request: &Request<'_>) -> Result<Response, HttpError> {
    let mut stream = TcpStream::connect(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)).await?;
    stream.set_nodelay(true)?;
    let mut head = format!(
        "{} {} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {}\r\n\
         Connection: close\r\nAccept: */*\r\n",
        request.method, request.path, request.key
    );
    if let Some(body) = request.body {
        head.push_str("Content-Type: application/json\r\n");
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await?;
    if let Some(body) = request.body {
        stream.write_all(body).await?;
    }
    stream.flush().await?;

    let mut buffer = Vec::with_capacity(4096);
    let end = loop {
        if let Some(at) = find(&buffer, b"\r\n\r\n") {
            break at;
        }
        if buffer.len() > MAX_HEAD_BYTES {
            return Err(HttpError::Invalid("header block too large"));
        }
        let mut chunk = [0u8; 4096];
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Err(HttpError::Invalid("connection closed before the headers"));
        }
        buffer.extend_from_slice(&chunk[..read]);
    };
    let head = std::str::from_utf8(&buffer[..end]).map_err(|_| HttpError::Invalid("headers"))?;
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or(HttpError::Invalid("status line"))?;
    let chunked = lines.any(|line| {
        let lower = line.to_ascii_lowercase();
        lower.starts_with("transfer-encoding:") && lower.contains("chunked")
    });
    let rest = buffer[end + 4..].to_vec();
    Ok(Response {
        stream,
        status,
        chunked,
        buffer: rest,
        chunk_left: 0,
        body_read: 0,
        done: false,
        pending: Vec::new(),
    })
}

impl Response {
    /// The whole body as text, at most `max` bytes (the rest is dropped).
    pub async fn text(mut self, max: usize) -> Result<String, HttpError> {
        let mut out = Vec::new();
        while let Some(bytes) = self.next_body().await? {
            let room = max.saturating_sub(out.len());
            out.extend_from_slice(&bytes[..bytes.len().min(room)]);
        }
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    /// Fail with (a short form of) the body unless the status is 2xx.
    pub async fn ok(self) -> Result<Self, HttpError> {
        if (200..300).contains(&self.status) {
            return Ok(self);
        }
        let status = self.status;
        let body = self.text(2048).await.unwrap_or_default();
        Err(HttpError::Status {
            status,
            body: super::one_line(&body, 300),
        })
    }

    /// The next line of the body without its line ending; `None` at the end.
    pub async fn next_line(&mut self) -> Result<Option<String>, HttpError> {
        loop {
            if let Some(at) = self.pending.iter().position(|&b| b == b'\n') {
                let mut line: Vec<u8> = self.pending.drain(..=at).collect();
                line.pop();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                return Ok(Some(String::from_utf8_lossy(&line).into_owned()));
            }
            if self.pending.len() > MAX_LINE_BYTES {
                return Err(HttpError::Invalid("line too long"));
            }
            match self.next_body().await? {
                Some(bytes) => self.pending.extend_from_slice(&bytes),
                None if self.pending.is_empty() => return Ok(None),
                None => {
                    let line = std::mem::take(&mut self.pending);
                    return Ok(Some(String::from_utf8_lossy(&line).into_owned()));
                }
            }
        }
    }

    /// The next piece of the decoded body; `None` at the end.
    async fn next_body(&mut self) -> Result<Option<Vec<u8>>, HttpError> {
        if self.done {
            return Ok(None);
        }
        let piece = if self.chunked {
            self.next_chunk_piece().await?
        } else {
            self.next_raw().await?
        };
        match &piece {
            Some(bytes) => {
                self.body_read += bytes.len();
                if self.body_read > MAX_BODY_BYTES {
                    return Err(HttpError::Invalid("body too large"));
                }
            }
            None => self.done = true,
        }
        Ok(piece)
    }

    async fn next_raw(&mut self) -> Result<Option<Vec<u8>>, HttpError> {
        if !self.buffer.is_empty() {
            return Ok(Some(std::mem::take(&mut self.buffer)));
        }
        let mut chunk = vec![0u8; 8192];
        let read = self.stream.read(&mut chunk).await?;
        if read == 0 {
            return Ok(None);
        }
        chunk.truncate(read);
        Ok(Some(chunk))
    }

    async fn next_chunk_piece(&mut self) -> Result<Option<Vec<u8>>, HttpError> {
        if self.chunk_left == 0 {
            let at = loop {
                if let Some(at) = find(&self.buffer, b"\r\n") {
                    break at;
                }
                if self.buffer.len() > 64 {
                    return Err(HttpError::Invalid("chunk size line"));
                }
                self.fill().await?;
            };
            let size = std::str::from_utf8(&self.buffer[..at])
                .ok()
                .and_then(|line| line.split(';').next())
                .and_then(|size| usize::from_str_radix(size.trim(), 16).ok())
                .ok_or(HttpError::Invalid("chunk size"))?;
            self.buffer.drain(..at + 2);
            if size == 0 {
                return Ok(None);
            }
            if size > MAX_BODY_BYTES {
                return Err(HttpError::Invalid("chunk too large"));
            }
            self.chunk_left = size;
        }
        if self.buffer.is_empty() {
            self.fill().await?;
        }
        let take = self.chunk_left.min(self.buffer.len());
        let piece: Vec<u8> = self.buffer.drain(..take).collect();
        self.chunk_left -= take;
        if self.chunk_left == 0 {
            while self.buffer.len() < 2 {
                self.fill().await?;
            }
            if &self.buffer[..2] != b"\r\n" {
                return Err(HttpError::Invalid("chunk terminator"));
            }
            self.buffer.drain(..2);
        }
        Ok(Some(piece))
    }

    async fn fill(&mut self) -> Result<(), HttpError> {
        let mut chunk = [0u8; 8192];
        let read = self.stream.read(&mut chunk).await?;
        if read == 0 {
            return Err(HttpError::Invalid("connection closed mid-body"));
        }
        self.buffer.extend_from_slice(&chunk[..read]);
        Ok(())
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use tokio::net::TcpListener;

    use super::*;

    async fn serve_once(reply: &'static [u8]) -> u16 {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut seen = Vec::new();
            let mut buf = [0u8; 1024];
            while find(&seen, b"\r\n\r\n").is_none() {
                let n = socket.read(&mut buf).await.unwrap();
                seen.extend_from_slice(&buf[..n]);
            }
            assert!(seen.starts_with(b"POST /x HTTP/1.1\r\n"));
            assert!(find(&seen, b"Authorization: Bearer k\r\n").is_some());
            socket.write_all(reply).await.unwrap();
        });
        port
    }

    fn request() -> Request<'static> {
        Request {
            method: "POST",
            path: "/x",
            key: "k",
            body: Some(b"{}"),
        }
    }

    #[tokio::test]
    async fn chunked_bodies_are_decoded_into_lines() {
        let port = serve_once(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
              7\r\ndata: a\r\n4\r\n\n\nda\r\n8\r\nta: [DON\r\n3\r\nE]\n\r\n0\r\n\r\n",
        )
        .await;
        let mut response = send(port, &request()).await.unwrap().ok().await.unwrap();
        let mut lines = Vec::new();
        while let Some(line) = response.next_line().await.unwrap() {
            lines.push(line);
        }
        assert_eq!(lines, ["data: a", "", "data: [DONE]"]);
    }

    #[tokio::test]
    async fn error_statuses_carry_a_short_body() {
        let port = serve_once(b"HTTP/1.1 503 Unavailable\r\n\r\nloading\r\nmodel").await;
        let error = send(port, &request())
            .await
            .unwrap()
            .ok()
            .await
            .err()
            .unwrap();
        assert!(
            matches!(&error, HttpError::Status { status: 503, body } if body == "loading model"),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn malformed_chunks_are_refused() {
        let port = serve_once(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n").await;
        let mut response = send(port, &request()).await.unwrap();
        assert!(response.next_line().await.is_err());
    }
}
