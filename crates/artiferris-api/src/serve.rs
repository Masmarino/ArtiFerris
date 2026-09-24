//! `axum::serve`, plus the header-read timeout it has no setting for: a client that opens a connection and never finishes its
//! request headers is dropped instead of holding the connection forever. HTTP/1 only: the auto builder's protocol sniffing
//! runs before any timer and would let a silent client sit there.

use std::future::Future;
use std::net::SocketAddr;
use std::pin::pin;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use hyper::body::Incoming;
use hyper::server::conn::http1::Builder;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::server::graceful::GracefulShutdown;
use hyper_util::service::TowerToHyperService;
use tokio::net::TcpListener;
use tower::{Service, ServiceExt};

/// Serves `app` until `shutdown` completes, then lets the requests in flight finish.
pub async fn serve(listener: TcpListener, app: Router, header_read_timeout: Duration, shutdown: impl Future<Output = ()>) {
    let mut make_service = app.into_make_service_with_connect_info::<SocketAddr>();
    let graceful = GracefulShutdown::new();
    let mut shutdown = pin!(shutdown);
    loop {
        let (stream, remote_addr) = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok(connection) => connection,
                Err(e) => {
                    // Usually out of file descriptors; backing off keeps this from spinning.
                    tracing::warn!("failed to accept a connection: {e}");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            },
            () = &mut shutdown => break,
        };
        let service = make_service.call(remote_addr).await.unwrap_or_else(|never| match never {});
        let service = TowerToHyperService::new(service.map_request(|request: Request<Incoming>| request.map(Body::new)));
        let mut builder = Builder::new();
        builder.timer(TokioTimer::new()).header_read_timeout(header_read_timeout);
        let connection = graceful.watch(builder.serve_connection(TokioIo::new(stream), service));
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                tracing::debug!("connection from {remote_addr} ended with an error: {e}");
            }
        });
    }
    graceful.shutdown().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::get;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    async fn start(header_read_timeout: Duration) -> (SocketAddr, tokio::sync::oneshot::Sender<()>, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let app = Router::new().route("/", get(|| async { "hello" }));
        let server = tokio::spawn(serve(listener, app, header_read_timeout, async move {
            let _ = stopped.await;
        }));
        (addr, stop, server)
    }

    async fn read_to_end(stream: &mut TcpStream) -> String {
        let mut response = Vec::new();
        let _ = stream.read_to_end(&mut response).await;
        String::from_utf8_lossy(&response).into_owned()
    }

    #[tokio::test]
    async fn a_complete_request_is_answered() {
        let (addr, stop, server) = start(Duration::from_secs(5)).await;
        let mut client = TcpStream::connect(addr).await.unwrap();
        client.write_all(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").await.unwrap();

        let response = read_to_end(&mut client).await;

        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.ends_with("hello"), "{response}");
        let _ = stop.send(());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn a_client_that_never_finishes_its_headers_is_dropped() {
        let (addr, stop, server) = start(Duration::from_millis(200)).await;
        let mut client = TcpStream::connect(addr).await.unwrap();
        client.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n").await.unwrap();

        let outcome = tokio::time::timeout(Duration::from_secs(3), read_to_end(&mut client)).await;

        assert!(outcome.is_ok(), "the connection was still open after the header timeout");
        let _ = stop.send(());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn a_client_that_sends_nothing_is_dropped() {
        let (addr, stop, server) = start(Duration::from_millis(200)).await;
        let mut client = TcpStream::connect(addr).await.unwrap();

        let outcome = tokio::time::timeout(Duration::from_secs(3), read_to_end(&mut client)).await;

        assert!(outcome.is_ok(), "a silent connection was still open after the header timeout");
        let _ = stop.send(());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn a_client_that_sends_a_single_byte_is_dropped() {
        let (addr, stop, server) = start(Duration::from_millis(200)).await;
        let mut client = TcpStream::connect(addr).await.unwrap();
        client.write_all(b"P").await.unwrap();

        let outcome = tokio::time::timeout(Duration::from_secs(3), read_to_end(&mut client)).await;

        assert!(outcome.is_ok(), "a connection that stopped after one byte was still open after the header timeout");
        let _ = stop.send(());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn the_http2_preface_is_not_served() {
        let (addr, stop, server) = start(Duration::from_secs(5)).await;
        let mut client = TcpStream::connect(addr).await.unwrap();
        client.write_all(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n").await.unwrap();

        let response = tokio::time::timeout(Duration::from_secs(3), read_to_end(&mut client)).await.unwrap();

        assert!(!response.contains("200"), "h2c must not be accepted: {response}");
        let _ = stop.send(());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn the_client_address_still_reaches_the_handlers() {
        use axum::extract::ConnectInfo;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let app = Router::new().route("/", get(|ConnectInfo(peer): ConnectInfo<SocketAddr>| async move { peer.ip().to_string() }));
        let server = tokio::spawn(serve(listener, app, Duration::from_secs(5), async move {
            let _ = stopped.await;
        }));
        let mut client = TcpStream::connect(addr).await.unwrap();
        client.write_all(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").await.unwrap();

        let response = read_to_end(&mut client).await;

        assert!(response.ends_with("127.0.0.1"), "{response}");
        let _ = stop.send(());
        server.await.unwrap();
    }
}
