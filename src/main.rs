use futures_util::{SinkExt, StreamExt};
use scrap::{Capturer, Display};
use std::{net::SocketAddr, time::Duration};
use tokio::{
    io::{AsyncWrite, AsyncWriteExt},
    net::{
        tcp::{ReadHalf, WriteHalf},
        TcpListener, TcpStream,
    },
    select,
};

use tokio_tungstenite::{
    accept_async, accept_hdr_async, connect_async,
    tungstenite::{
        handshake::server::{Callback, ErrorResponse, Request, Response},
        http::HeaderValue,
        Error, Message, Result,
    },
};

pub struct WebSock;

impl Callback for WebSock {
    fn on_request(
        self,
        req: &Request,
        mut res: Response,
    ) -> std::result::Result<Response, ErrorResponse> {
        panic!();
        if req.headers().get("Upgrade").map(|s| s.as_ref()) == Some(b"websocket") {
            println!("YAY");
        }
        res.headers_mut()
            .insert("Connection", HeaderValue::from_static("upgrade"));
        Ok(res)
    }
}

async fn send_screen(
    capturer: &mut Capturer,
    sender: &mut futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
        tokio_tungstenite::tungstenite::Message,
    >,
    w: usize,
    h: usize,
    one_frame: Duration,
) {
    let now = std::time::Instant::now();
    let mut bitflipped = Vec::with_capacity(w * h * 4);
    loop {
        println!(
            "from `now` {}ms",
            (std::time::Instant::now() - now).as_millis()
        );
        let buffer = match capturer.frame() {
            Ok(buffer) => buffer,
            Err(error) => {
                if error.kind() == std::io::ErrorKind::WouldBlock {
                    // Keep spinning.
                    tokio::time::delay_for(one_frame);
                    println!(
                        "after `error sleep` {}ms",
                        (std::time::Instant::now() - now).as_millis()
                    );
                    continue;
                } else {
                    panic!("Error: {}", error);
                }
            }
        };
        // Write the frame, removing end-of-row padding.
        // Flip the ARGB image into a BGRA image.
        let stride = buffer.len() / h;
        for y in 0..h {
            for x in 0..w {
                let i = stride * y + 4 * x;
                bitflipped.extend_from_slice(&[buffer[i + 2], buffer[i + 1], buffer[i], 255]);
            }
        }

        println!(
            "after `argb - bgra` {}ms",
            (std::time::Instant::now() - now).as_millis()
        );

        sender
            .send(Message::Binary(
                serde_json::json!({
                    "chunks": bitflipped,
                    "width": w,
                    "height": h,
                })
                .to_string()
                .as_bytes()
                .to_vec(),
            ))
            .await
            .unwrap();
    }
}

async fn accept_connection(
    peer: SocketAddr,
    stream: TcpStream,
    capturer: &mut Capturer,
    w: usize,
    h: usize,
    one_frame: Duration,
) {
    if let Err(e) = handle_connection(peer, stream, capturer, w, h, one_frame).await {
        match e {
            Error::ConnectionClosed | Error::Protocol(_) | Error::Utf8 => (),
            err => eprintln!("Error processing connection: {}", err),
        }
    }
}

async fn handle_connection(
    peer: SocketAddr,
    stream: TcpStream,
    capturer: &mut Capturer,
    w: usize,
    h: usize,
    one_frame: Duration,
) -> Result<()> {
    let ws_stream = accept_hdr_async(stream, WebSock)
        .await
        .expect("Failed to accept");

    println!("New WebSocket connection: {}", peer);
    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    let now = std::time::Instant::now();

    let mut delay = tokio::time::delay_for(Duration::from_millis(50));
    if let Some(Ok(x)) = ws_receiver.next().await {
        println!("{}", x);
        ws_sender.send(x).await.unwrap();
    };

    loop {
        tokio::select! {
            _ = send_screen(capturer, &mut ws_sender, w, h, one_frame) => {},
            _ = &mut delay => {},
            else => break,
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() {
    let addr = "127.0.0.1:8080";
    let mut listener = TcpListener::bind(&addr).await.expect("Can't listen");
    println!("Listening on: {}", addr);

    let one_second = std::time::Duration::new(1, 0);
    let one_frame = one_second / 60;

    let d = Display::primary().unwrap();
    let (w, h) = (d.width(), d.height());

    let mut capturer = Capturer::new(d).unwrap();

    while let Ok((stream, _)) = listener.accept().await {
        let peer = stream
            .peer_addr()
            .expect("connected streams should have a peer address");
        println!("Peer address: {}", peer);

        accept_connection(peer, stream, &mut capturer, w, h, one_frame).await;
    }
}
