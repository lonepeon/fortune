use std::sync::Arc;
use std::sync::RwLock;
use std::time;

use clap::{Args, Parser, Subcommand};
use rand::{seq::SliceRandom, SeedableRng};
use tokio::io::AsyncWriteExt;
use tokio::signal;
use tokio::sync::oneshot::Receiver;
use tokio::sync::oneshot::Sender;

#[derive(Args)]
struct CliCommandServer {
    #[arg(
        long = "listen-tcp",
        short = 't',
        help = "Bind server to given TCP address",
        default_value = "127.0.0.1:8080"
    )]
    tcp_address: String,
    #[arg(
        long = "listen-udp",
        short = 'u',
        help = "Bind server to given UDP address",
        default_value = "127.0.0.1:8081"
    )]
    udp_address: String,
}

#[derive(Subcommand)]
#[command()]
enum CliCommand {
    #[command(
        name = "server",
        about = "Start a Quote Of The Day TCP and UDP server (RFC 865)

- TCP server: for each new TCP connection, it sends a quote
- UDP server: for each datagram received, it sends a quote

Both servers ignore the content of the packet, they blindly send quotes.
"
    )]
    Server(CliCommandServer),
    #[command(name = "generate", about = "Prints a Quote Of The Day message")]
    Generate,
}

#[derive(Parser)]
#[command(author, version, about)]
struct Cli {
    #[arg(long = "seed", help = "Seed value to initialize randomizer")]
    seed: Option<u64>,
    #[command(subcommand)]
    command: CliCommand,
}

type AsyncQuoteGen<G> = Arc<RwLock<QuoteGen<G>>>;

struct QuoteGen<G: rand::Rng + Send + Sync> {
    gen: G,
    quotes: Vec<&'static str>,
}

unsafe impl<G: rand::Rng + Send + Sync> std::marker::Send for QuoteGen<G> {}

impl<G: rand::Rng + Send + Sync> QuoteGen<G> {
    fn generate(&mut self) -> &'static str {
        self.quotes.choose(&mut self.gen).unwrap()
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let quotes: Vec<_> = include_str!("../quotes.txt")
        .split_inclusive('\n')
        .collect();

    if let Some(graceful_shutdown) = run(std::io::stdout(), cli, quotes).await {
        signal::ctrl_c().await.unwrap();

        graceful_shutdown.stop().await;
    }
}

struct ServerStopper {
    ask_stop: tokio::sync::watch::Sender<bool>,
    wait_tcp_stopped: Receiver<()>,
    wait_udp_stopped: Receiver<()>,
}

impl ServerStopper {
    async fn stop(self) {
        println!("stopping TCP and UDP servers...");
        self.ask_stop.send(true).unwrap();
        self.wait_tcp_stopped.await.unwrap();
        self.wait_udp_stopped.await.unwrap();
    }
}

async fn run<W: std::io::Write>(
    mut w: W,
    cli: Cli,
    quotes: Vec<&'static str>,
) -> Option<ServerStopper> {
    let seed = cli.seed.unwrap_or_else(|| {
        time::SystemTime::now()
            .duration_since(time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64
    });

    let gen = rand::rngs::StdRng::seed_from_u64(seed);
    let quotes = Arc::new(RwLock::new(QuoteGen { gen, quotes }));

    match cli.command {
        CliCommand::Generate => {
            write!(w, "{}", quotes.write().unwrap().generate()).unwrap();
            None
        }
        CliCommand::Server(opts) => {
            let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
            let (tcp_server_stopped_tx, tcp_server_stopped_rx): (Sender<()>, Receiver<()>) =
                tokio::sync::oneshot::channel();
            let (udp_server_stopped_tx, udp_server_stopped_rx): (Sender<()>, Receiver<()>) =
                tokio::sync::oneshot::channel();

            let tcp_server = spawn_tcp_server(
                opts.tcp_address,
                tcp_server_stopped_tx,
                stop_rx.clone(),
                quotes.clone(),
            );

            let udp_server = spawn_udp_server(
                opts.udp_address,
                udp_server_stopped_tx,
                stop_rx.clone(),
                quotes.clone(),
            );

            tcp_server.await;
            udp_server.await;

            Some(ServerStopper {
                ask_stop: stop_tx,
                wait_tcp_stopped: tcp_server_stopped_rx,
                wait_udp_stopped: udp_server_stopped_rx,
            })
        }
    }
}

async fn spawn_tcp_server<R: rand::Rng + Send + Sync + 'static>(
    addr: String,
    stopped_signal: tokio::sync::oneshot::Sender<()>,
    mut stop_signal: tokio::sync::watch::Receiver<bool>,
    quotes: AsyncQuoteGen<R>,
) -> tokio::task::JoinHandle<()> {
    let server = tokio::net::TcpListener::bind(&addr).await.unwrap();
    println!("TCP server listening on {}", addr);

    tokio::spawn(async move {
        loop {
            tokio::select! {
                stream = server.accept()=> {
                    let (mut socket,_) = stream.unwrap();
                    let quote = quotes.write().unwrap().generate();
                    socket.write_all(quote.as_bytes()).await.unwrap();
                }
                _ = stop_signal.changed() => {
                    stopped_signal.send(()).unwrap();
                    return;
                }
            }
        }
    })
}

async fn spawn_udp_server<R: rand::Rng + Send + Sync + 'static>(
    addr: String,
    stopped_signal: tokio::sync::oneshot::Sender<()>,
    mut stop_signal: tokio::sync::watch::Receiver<bool>,
    quotes: AsyncQuoteGen<R>,
) -> tokio::task::JoinHandle<()> {
    let server = tokio::net::UdpSocket::bind(&addr).await.unwrap();
    println!("UDP server listening on {}", addr);
    let mut recv = [0; 1];

    tokio::spawn(async move {
        loop {
            tokio::select! {
                Ok((_, dest)) = server.recv_from(&mut recv) => {
                    let quote = quotes.write().unwrap().generate();
                    server.send_to(quote.as_bytes(), dest).await.unwrap();
                }
                _ = stop_signal.changed() => {
                    stopped_signal.send(()).unwrap();
                    return;
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn generate() {
        let quotes = vec!["my quote of the day\n"];

        let cli = super::Cli {
            seed: Some(1),
            command: crate::CliCommand::Generate,
        };

        let mut buf = Vec::new();

        assert!(
            super::run(&mut buf, cli, quotes).await.is_none(),
            "did not expect cleanup function"
        );

        assert_eq!("my quote of the day\n".as_bytes(), buf)
    }

    #[tokio::test]
    async fn udp_server() {
        let quotes = vec!["my quote of the day\n"];

        let udp_address: std::net::SocketAddr = "127.0.0.1:10004"
            .parse()
            .expect("failed to build server address");

        let cli = super::Cli {
            seed: Some(1),
            command: crate::CliCommand::Server(super::CliCommandServer {
                tcp_address: "127.0.0.1:10003".to_string(),
                udp_address: udp_address.to_string(),
            }),
        };

        let mut stdout = Vec::new();
        let stopper = super::run(&mut stdout, cli, quotes)
            .await
            .expect("failed to get server stopping function");

        let client = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("failed to bind UDP client");
        client
            .connect(udp_address)
            .await
            .expect("failed to connect to server");

        client
            .send("".as_bytes())
            .await
            .expect("failed to send UDP datagram");
        let mut buf = [0; 20];
        client
            .recv(&mut buf)
            .await
            .expect("failed to receive datagram");

        assert_eq!("my quote of the day\n".as_bytes(), buf);

        stopper.stop().await;
    }

    #[tokio::test]
    async fn tcp_server() {
        let quotes = vec!["my quote of the day\n"];

        let tcp_address: std::net::SocketAddr = "127.0.0.1:10001"
            .parse()
            .expect("failed to build server address");

        let cli = super::Cli {
            seed: Some(1),
            command: crate::CliCommand::Server(super::CliCommandServer {
                tcp_address: tcp_address.to_string(),
                udp_address: "127.0.0.1:10002".to_string(),
            }),
        };

        let mut stdout = Vec::new();
        let stopper = super::run(&mut stdout, cli, quotes)
            .await
            .expect("failed to get server stopping function");

        let client = tokio::net::TcpSocket::new_v4().expect("failed to create client TCP socket");
        let mut client_stream = client
            .connect(tcp_address)
            .await
            .expect("failed to connect to server");

        client_stream
            .write_all("".as_bytes())
            .await
            .expect("failed to send TCP message");

        let mut buf = [0; 20];
        client_stream
            .read(&mut buf)
            .await
            .expect("failed to receive TCP message");

        assert_eq!("my quote of the day\n".as_bytes(), buf);

        stopper.stop().await;
    }
}
