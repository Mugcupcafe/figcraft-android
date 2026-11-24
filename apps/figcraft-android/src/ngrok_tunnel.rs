//! In-process Rust ngrok HTTPS tunnel manager for FigCraft Mobile (V28).
//! Uses official `ngrok` Rust SDK crate (`ngrok-rs`) so DNS runs natively through Android Bionic
//! and TLS uses embedded ngrok root CAs — zero Go `/etc/resolv.conf` issues.

use ngrok::prelude::*;
use std::sync::mpsc;
use std::thread;

#[derive(Debug, Clone)]
pub enum NgrokEvent {
    Connecting(String),
    Connected { mcp_url: String },
    Error(String),
    Stopped,
}

pub struct NgrokTunnelHandle {
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    pub rx: mpsc::Receiver<NgrokEvent>,
}

impl NgrokTunnelHandle {
    pub fn start(authtoken: String, domain: String, local_port: u16) -> Self {
        let (ev_tx, ev_rx) = mpsc::channel::<NgrokEvent>();
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();

        let token = authtoken.trim().to_owned();
        let dom = domain.trim().to_owned();
        let _ = ev_tx.send(NgrokEvent::Connecting("ngrok 연결 중...".into()));

        thread::Builder::new()
            .name("figcraft-ngrok".into())
            .spawn(move || {
                if token.is_empty() {
                    let _ = ev_tx.send(NgrokEvent::Error("ngrok Authtoken을 먼저 입력해 주세요".into()));
                    return;
                }
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(r) => r,
                    Err(e) => {
                        let _ = ev_tx.send(NgrokEvent::Error(format!("ngrok 런타임 오류: {e}")));
                        return;
                    }
                };

                rt.block_on(async move {
                    let target_url = match url::Url::parse(&format!("http://127.0.0.1:{local_port}")) {
                        Ok(u) => u,
                        Err(e) => {
                            let _ = ev_tx.send(NgrokEvent::Error(format!("로컬 주소 오류: {e}")));
                            return;
                        }
                    };

                    let session = match ngrok::Session::builder()
                        .authtoken(token)
                        .connect()
                        .await
                    {
                        Ok(s) => s,
                        Err(e) => {
                            let _ = ev_tx.send(NgrokEvent::Error(format!("ngrok 로그인 실패: {e}")));
                            return;
                        }
                    };

                    let mut ep = session.http_endpoint();
                    if !dom.is_empty() {
                        let clean_dom = dom
                            .trim_start_matches("https://")
                            .trim_start_matches("http://")
                            .trim_end_matches('/')
                            .to_owned();
                        ep.domain(clean_dom);
                    }

                    let mut forwarder = match ep.listen_and_forward(target_url).await {
                        Ok(f) => f,
                        Err(e) => {
                            let _ = ev_tx.send(NgrokEvent::Error(format!("ngrok 터널 생성 실패: {e}")));
                            return;
                        }
                    };

                    let base = forwarder.url().trim_end_matches('/').to_owned();
                    let mcp_url = format!("{base}/mcp");
                    let _ = ev_tx.send(NgrokEvent::Connected { mcp_url });

                    // Keep forwarding until shutdown requested or forwarder exits
                    let stop_hit = tokio::select! {
                        _ = stop_rx => true,
                        res = forwarder.join() => {
                            if let Err(e) = res {
                                let _ = ev_tx.send(NgrokEvent::Error(format!("ngrok 연결 끊김: {e}")));
                                return;
                            }
                            false
                        }
                    };
                    if stop_hit {
                        let _ = forwarder.close().await;
                    }
                    let _ = ev_tx.send(NgrokEvent::Stopped);
                });
            });

        Self {
            shutdown_tx: Some(stop_tx),
            rx: ev_rx,
        }
    }

    pub fn stop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for NgrokTunnelHandle {
    fn drop(&mut self) {
        self.stop();
    }
}
