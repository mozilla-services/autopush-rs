//! I/O wrapper created through `Dispatch`
//!
//! Most I/O happens through just raw TCP sockets, but at the beginning of a
//! request we'll take a look at the headers and figure out where to route it.
//! After that, for tungstenite the websocket library, we'll want to replay the
//! data we already read as there's no ability to pass this in currently. That
//! means we'll parse headers twice, but alas!

use std::io::{self, Read, Write};

use bytes::BytesMut;
use futures::Poll;
use tokio_core::net::TcpStream;
use tokio_io::{AsyncRead, AsyncWrite};

use server::tls::MaybeTlsStream;

pub struct WebpushIo {
    tcp: MaybeTlsStream<TcpStream>,
    header_to_read: Option<BytesMut>,
}

impl WebpushIo {
    pub fn new(tcp: MaybeTlsStream<TcpStream>, header: BytesMut) -> Self {
        Self {
            tcp: tcp,
            header_to_read: Some(header),
        }
    }
}

impl Read for WebpushIo {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // Start off by replaying the bytes already read, and after that just
        // delegate everything to the internal `TcpStream`
        if let Some(ref mut header) = self.header_to_read {
            let n = (&header[..]).read(buf)?;
            header.split_to(n);
            if buf.is_empty() || n > 0 {
                return Ok(n);
            }
        }
        self.header_to_read = None;
        self.tcp.read(buf)
    }
}

// All `write` calls are routed through the `TcpStream` instance directly as we
// don't buffer this at all.
impl Write for WebpushIo {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.tcp.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.tcp.flush()
    }
}

impl AsyncRead for WebpushIo {}

impl AsyncWrite for WebpushIo {
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        AsyncWrite::shutdown(&mut self.tcp)
    }
}
