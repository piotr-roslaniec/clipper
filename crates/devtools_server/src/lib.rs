//! A server for Chrome devtools protocol.
//!
//! Used to provide the network tab's APIs.

use std::{
    marker::PhantomData,
    net::SocketAddr,
    pin::Pin,
    task::{Context, Poll},
};

use chromiumoxide_types::{CallId, EventMessage};
use futures::{future::BoxFuture, SinkExt, Stream};
use std::future::Future;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{tungstenite, WebSocketStream};

pub use chromiumoxide_cdp as cdp;
pub use chromiumoxide_types as cdp_types;

pub const METHOD_NOT_FOUND: i64 = -32601;

pub type Error = Box<dyn std::error::Error + Send + Sync>;

type Next = Option<
    BoxFuture<'static, Result<WebSocketStream<TcpStream>, tokio_tungstenite::tungstenite::Error>>,
>;

pub struct ConnectionStream {
    listener: TcpListener,
    next: Next,
}

impl ConnectionStream {
    pub async fn new(sa: SocketAddr) -> Result<Self, Error> {
        let listener = TcpListener::bind(sa).await?;

        Ok(Self {
            listener,
            next: None,
        })
    }
    fn next_mut(self: Pin<&mut Self>) -> Pin<&mut Next> {
        // SAFETY: next is considered structurally pinned
        unsafe { self.map_unchecked_mut(|this| &mut this.next) }
    }
}

impl Stream for ConnectionStream {
    type Item = Result<ServerConnection, Error>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        // XXX: holy hell, am i bad at programming or is it simply horrifying
        // to hand roll futures?

        loop {
            if let Some(p) = self.as_mut().next_mut().as_pin_mut() {
                match Future::poll(p, cx) {
                    Poll::Ready(r) => {
                        *self.as_mut().next_mut() = None;
                        let wss = r?;
                        return Poll::Ready(Some(Ok(ServerConnection::new(wss))));
                    }
                    Poll::Pending => return Poll::Pending,
                }
            } else {
                match self.listener.poll_accept(cx)? {
                    Poll::Ready((stream, _sa)) => {
                        let ws_stream = tokio_tungstenite::accept_async(stream);
                        *self.as_mut().next_mut() = Some(Box::pin(ws_stream));
                        // due to poll safety: cannot return Pending without
                        // registering a waker
                        continue;
                    }
                    Poll::Pending => return Poll::Pending,
                }
            }
        }
    }
}

pub struct ServerConnection {
    wss: WebSocketStream<TcpStream>,
}

impl ServerConnection {
    fn new(wss: WebSocketStream<TcpStream>) -> Self {
        Self { wss }
    }

    pub async fn reply(
        &mut self,
        id: CallId,
        result: impl Into<serde_json::Value>,
    ) -> Result<(), Error> {
        self.send(cdp_types::Message::Response(cdp_types::Response {
            id,
            result: Some(result.into()),
            error: None,
        }))
        .await
    }

    pub async fn send(&mut self, response: chromiumoxide_types::Message) -> Result<(), Error> {
        let text = serde_json::to_vec(&response)?;

        tracing::debug!("send: {}", hexdump::HexDumper::new(&text));
        self.wss
            .send(tungstenite::Message::Text(String::from_utf8(text).unwrap()))
            .await?;

        Ok(())
    }

    fn wss_mut(self: Pin<&mut Self>) -> Pin<&mut WebSocketStream<TcpStream>> {
        // SAFETY: wss is considered structurally pinned
        unsafe { self.map_unchecked_mut(|this| &mut this.wss) }
    }
}

impl Stream for ServerConnection {
    type Item = Result<chromiumoxide_types::MethodCall, Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match Stream::poll_next(self.wss_mut(), cx) {
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Ready(Some(res)) => {
                let msg = res?;
                let data = msg.into_data();
                tracing::debug!("message: {}", hexdump::HexDumper::new(&data));
                let msg = serde_json::from_slice(&data)?;

                Poll::Ready(Some(Ok(msg)))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}
