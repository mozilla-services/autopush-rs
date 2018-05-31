//! TLS support for the autopush server
//!
//! Currently tungstenite in the way we use it just operates generically over an
//! `AsyncRead`/`AsyncWrite` stream, so this provides a `MaybeTlsStream` type
//! which dispatches at runtime whether it's a plaintext or TLS stream after a
//! connection is established.

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;
use std::rc::Rc;

use futures::future;
use futures::{Future, Poll};
use openssl::dh::Dh;
use openssl::pkey::PKey;
use openssl::ssl::{SslAcceptor, SslMethod, SslMode};
use openssl::x509::X509;
use tokio_core::net::TcpStream;
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_openssl::{SslAcceptorExt, SslStream};

use errors::*;
use server::{Server, ServerOptions};

/// Creates an `SslAcceptor`, if needed, ready to accept TLS connections.
///
/// This method is called early on when the server is created and the
/// `SslAcceptor` type is stored globally in the `Server` structure, later used
/// to process all accepted TCP sockets.
pub fn configure(opts: &ServerOptions) -> Option<SslAcceptor> {
    let key = match opts.ssl_key {
        Some(ref key) => read(key),
        None => return None,
    };
    let key = PKey::private_key_from_pem(&key).expect("failed to create private key");
    let cert = read(opts.ssl_cert.as_ref().expect("ssl_cert not configured"));
    let cert = X509::from_pem(&cert).expect("failed to create certificate");

    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls())
        .expect("failed to create ssl acceptor builder");
    builder
        .set_private_key(&key)
        .expect("failed to set private key");
    builder
        .set_certificate(&cert)
        .expect("failed to set certificate");
    builder
        .check_private_key()
        .expect("private key check failed");

    if let Some(dh_param) = opts.ssl_dh_param.as_ref() {
        let dh_param = Dh::params_from_pem(&read(dh_param)).expect("failed to create dh");
        builder
            .set_tmp_dh(&dh_param)
            .expect("failed to set dh_param");
    }

    // Should help reduce peak memory consumption for idle connections
    builder.set_mode(SslMode::RELEASE_BUFFERS);

    return Some(builder.build());

    fn read(path: &Path) -> Vec<u8> {
        let mut out = Vec::new();
        File::open(path)
            .expect(&format!("failed to open {:?}", path))
            .read_to_end(&mut out)
            .expect(&format!("failed to read {:?}", path));
        out
    }
}

/// Performs the TLS handshake, if necessary, for a socket.
///
/// This is typically called just after a socket has been accepted from the TCP
/// listener. If the server is configured without TLS then this will immediately
/// return with a plaintext socket, or otherwise it will perform an asynchronous
/// TLS handshake and only resolve once that's completed.
pub fn accept(srv: &Rc<Server>, socket: TcpStream) -> MyFuture<MaybeTlsStream<TcpStream>> {
    match srv.tls_acceptor {
        Some(ref acceptor) => Box::new(
            acceptor
                .accept_async(socket)
                .map(MaybeTlsStream::Tls)
                .chain_err(|| "failed to accept TLS socket"),
        ),
        None => Box::new(future::ok(MaybeTlsStream::Plain(socket))),
    }
}

pub enum MaybeTlsStream<T> {
    Plain(T),
    Tls(SslStream<T>),
}

impl<T: Read + Write> Read for MaybeTlsStream<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match *self {
            MaybeTlsStream::Plain(ref mut s) => s.read(buf),
            MaybeTlsStream::Tls(ref mut s) => s.read(buf),
        }
    }
}

impl<T: Read + Write> Write for MaybeTlsStream<T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match *self {
            MaybeTlsStream::Plain(ref mut s) => s.write(buf),
            MaybeTlsStream::Tls(ref mut s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match *self {
            MaybeTlsStream::Plain(ref mut s) => s.flush(),
            MaybeTlsStream::Tls(ref mut s) => s.flush(),
        }
    }
}

impl<T: AsyncRead + AsyncWrite> AsyncRead for MaybeTlsStream<T> {
    unsafe fn prepare_uninitialized_buffer(&self, buf: &mut [u8]) -> bool {
        match *self {
            MaybeTlsStream::Plain(ref s) => s.prepare_uninitialized_buffer(buf),
            MaybeTlsStream::Tls(ref s) => s.prepare_uninitialized_buffer(buf),
        }
    }
}

impl<T: AsyncWrite + AsyncRead> AsyncWrite for MaybeTlsStream<T> {
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        match *self {
            MaybeTlsStream::Plain(ref mut s) => s.shutdown(),
            MaybeTlsStream::Tls(ref mut s) => s.shutdown(),
        }
    }
}
