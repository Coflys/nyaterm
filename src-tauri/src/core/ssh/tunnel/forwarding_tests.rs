//! Exercise the actual forwarding loops against a local SSH test server.
use super::*;
use russh::{client, server};
use std::sync::atomic::AtomicUsize;
use tokio::time::timeout;

type TestHandle = Arc<Mutex<client::Handle<TestClient>>>;

struct TestClient(mpsc::UnboundedSender<RemoteForwardOpen>);
impl client::Handler for TestClient {
    type Error = russh::Error;
    async fn check_server_key(&mut self, _: &russh::keys::PublicKey) -> Result<bool, Self::Error> {
        Ok(true)
    }
    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: russh::Channel<client::Msg>,
        address: &str,
        port: u32,
        origin: &str,
        origin_port: u32,
        reply: client::ChannelOpenHandle,
        _: &mut client::Session,
    ) -> Result<(), Self::Error> {
        self.0
            .send(RemoteForwardOpen {
                channel,
                connected_address: address.into(),
                connected_port: port,
                originator_address: origin.into(),
                originator_port: origin_port,
            })
            .unwrap_or_else(|_| panic!("forward receiver"));
        reply.accept().await;
        Ok(())
    }
}

struct TestServer(AtomicUsize);
impl server::Handler for TestServer {
    type Error = russh::Error;
    async fn auth_none(&mut self, _: &str) -> Result<server::Auth, Self::Error> {
        Ok(server::Auth::Accept)
    }
    async fn tcpip_forward(
        &mut self,
        _: &str,
        _: &mut u32,
        _: &mut server::Session,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
    async fn channel_open_direct_tcpip(
        &mut self,
        channel: russh::Channel<server::Msg>,
        _: &str,
        _: u32,
        _: &str,
        _: u32,
        reply: server::ChannelOpenHandle,
        _: &mut server::Session,
    ) -> Result<(), Self::Error> {
        if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
            reply.reject(russh::ChannelOpenFailure::ConnectFailed).await;
        } else {
            reply.accept().await;
            tokio::spawn(async move {
                let mut stream = channel.into_stream();
                let mut buffer = [0; 64];
                while let Ok(size) = stream.read(&mut buffer).await {
                    if size == 0 || stream.write_all(&buffer[..size]).await.is_err() {
                        break;
                    }
                }
            });
        }
        Ok(())
    }
}

async fn connect() -> (
    TestHandle,
    server::Handle,
    JoinHandle<()>,
    mpsc::UnboundedReceiver<RemoteForwardOpen>,
) {
    let (client_io, server_io) = tokio::io::duplex(65536);
    let (server_tx, server_rx) = tokio::sync::oneshot::channel();
    let config = Arc::new(server::Config {
        keys: vec![
            russh::keys::PrivateKey::random(
                &mut russh::keys::key::safe_rng(),
                russh::keys::Algorithm::Ed25519,
            )
            .unwrap(),
        ],
        auth_rejection_time: Duration::ZERO,
        auth_rejection_time_initial: Some(Duration::ZERO),
        ..Default::default()
    });
    let server_task = tokio::spawn(async move {
        let session = server::run_stream(config, server_io, TestServer(AtomicUsize::new(0)))
            .await
            .unwrap();
        server_tx.send(session.handle()).unwrap();
        let _ = session.await;
    });
    let (remote_tx, remote_rx) = mpsc::unbounded_channel();
    let mut client = client::connect_stream(
        Arc::new(client::Config::default()),
        client_io,
        TestClient(remote_tx),
    )
    .await
    .unwrap();
    assert!(matches!(
        client.authenticate_none("test").await.unwrap(),
        client::AuthResult::Success
    ));
    (
        Arc::new(Mutex::new(client)),
        server_rx.await.unwrap(),
        server_task,
        remote_rx,
    )
}

async fn stop_ssh(client: TestHandle, server: JoinHandle<()>) {
    client
        .lock()
        .await
        .disconnect(russh::Disconnect::ByApplication, "test complete", "")
        .await
        .unwrap();
    drop(client);
    timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
}

async fn echo(stream: &mut (impl tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin)) {
    stream.write_all(b"hello").await.unwrap();
    let mut response = [0; 5];
    timeout(Duration::from_secs(2), stream.read_exact(&mut response))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&response, b"hello");
}

#[tokio::test]
async fn local_tunnel_survives_a_failed_request_and_releases_listener_and_clients_on_stop() {
    let (client, _, server, _) = connect().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let cancel = CancellationToken::new();
    let task_cancel = cancel.clone();
    let task_client = client.clone();
    let (_disconnect_tx, disconnect_rx) = mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        let mut children = JoinSet::new();
        let exit = TunnelManager::run_local_tunnel(
            listener,
            task_client,
            "target".into(),
            80,
            task_cancel,
            &mut children,
            disconnect_rx,
        )
        .await;
        children.abort_all();
        while children.join_next().await.is_some() {}
        exit
    });
    let mut first = TcpStream::connect(addr).await.unwrap();
    assert_eq!(
        timeout(Duration::from_secs(5), first.read(&mut [0]))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    assert!(!task.is_finished());
    let mut second = TcpStream::connect(addr).await.unwrap();
    echo(&mut second).await;
    cancel.cancel();
    assert!(matches!(
        timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap(),
        TunnelTaskExit::Stopped
    ));
    assert_eq!(second.read(&mut [0]).await.unwrap(), 0);
    let rebound = TcpListener::bind(addr).await.unwrap();
    drop(rebound);
    stop_ssh(client, server).await;
}

#[tokio::test]
async fn dynamic_tunnel_survives_a_failed_socks_request_and_forwards_the_next() {
    let (client, _, server, _) = connect().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let cancel = CancellationToken::new();
    let task_cancel = cancel.clone();
    let task_client = client.clone();
    let (_disconnect_tx, disconnect_rx) = mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        let mut children = JoinSet::new();
        let exit = TunnelManager::run_dynamic_tunnel(
            listener,
            task_client,
            task_cancel,
            &mut children,
            disconnect_rx,
        )
        .await;
        children.abort_all();
        while children.join_next().await.is_some() {}
        exit
    });
    for expected in [5, 0] {
        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream.write_all(&[5, 1, 0]).await.unwrap();
        let mut greeting = [0; 2];
        stream.read_exact(&mut greeting).await.unwrap();
        assert_eq!(greeting, [5, 0]);
        stream
            .write_all(&[5, 1, 0, 1, 127, 0, 0, 1, 0, 80])
            .await
            .unwrap();
        let mut response = [0; 10];
        timeout(Duration::from_secs(2), stream.read_exact(&mut response))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(response[1], expected);
        if expected == 0 {
            echo(&mut stream).await;
        }
        assert!(!task.is_finished());
    }
    cancel.cancel();
    assert!(matches!(
        timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap(),
        TunnelTaskExit::Stopped
    ));
    TcpListener::bind(addr).await.unwrap();
    stop_ssh(client, server).await;
}

#[tokio::test]
async fn remote_tunnel_survives_an_unavailable_target_and_forwards_when_it_recovers() {
    let (client, server_handle, server, remote_rx) = connect().await;
    client
        .lock()
        .await
        .tcpip_forward("127.0.0.1", 9000)
        .await
        .unwrap();
    let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = target.local_addr().unwrap();
    drop(target);
    let cancel = CancellationToken::new();
    let task_cancel = cancel.clone();
    let (_disconnect_tx, disconnect_rx) = mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        let mut children = JoinSet::new();
        let exit = TunnelManager::run_remote_tunnel(
            "127.0.0.1".into(),
            addr.port(),
            task_cancel,
            &mut children,
            disconnect_rx,
            remote_rx,
        )
        .await;
        children.abort_all();
        while children.join_next().await.is_some() {}
        exit
    });
    let mut first = server_handle
        .channel_open_forwarded_tcpip("127.0.0.1", 9000, "127.0.0.1", 1000)
        .await
        .unwrap()
        .into_stream();
    assert_eq!(
        timeout(Duration::from_secs(5), first.read(&mut [0]))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    assert!(!task.is_finished());
    let target = TcpListener::bind(addr).await.unwrap();
    let target_task = tokio::spawn(async move {
        let (mut stream, _) = target.accept().await.unwrap();
        let mut data = [0; 5];
        stream.read_exact(&mut data).await.unwrap();
        stream.write_all(&data).await.unwrap();
    });
    let mut second = server_handle
        .channel_open_forwarded_tcpip("127.0.0.1", 9000, "127.0.0.1", 1001)
        .await
        .unwrap()
        .into_stream();
    echo(&mut second).await;
    target_task.await.unwrap();
    cancel.cancel();
    assert!(matches!(
        timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap(),
        TunnelTaskExit::Stopped
    ));
    stop_ssh(client, server).await;
}
