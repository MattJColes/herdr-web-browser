//! Minimal synchronous Chrome DevTools Protocol client (flattened sessions).

use crate::R;
use serde_json::{Value, json};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};
use tungstenite::{Message, WebSocket};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

pub struct Cdp {
    ws: WebSocket<TcpStream>,
    next_id: u64,
}

impl Cdp {
    pub fn connect(url: &str) -> R<Cdp> {
        let host = url
            .strip_prefix("ws://")
            .and_then(|r| r.split('/').next())
            .ok_or_else(|| format!("bad CDP url: {url}"))?;
        let addr = host.to_socket_addrs()?.next().ok_or("CDP host did not resolve")?;
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(3))?;
        stream.set_write_timeout(Some(Duration::from_secs(10)))?;
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        let (ws, _) = tungstenite::client(url, stream).map_err(|e| format!("CDP handshake: {e}"))?;
        Ok(Cdp { ws, next_id: 0 })
    }

    pub fn call(&mut self, method: &str, params: Value, session: Option<&str>) -> R<Value> {
        self.call_timeout(method, params, session, DEFAULT_TIMEOUT)
    }

    /// Send without waiting; the reply is skipped by later reads.
    pub fn send(&mut self, method: &str, params: Value, session: Option<&str>) -> R<u64> {
        self.next_id += 1;
        let mut msg = json!({ "id": self.next_id, "method": method, "params": params });
        if let Some(s) = session {
            msg["sessionId"] = json!(s);
        }
        self.ws.send(Message::text(msg.to_string()))?;
        Ok(self.next_id)
    }

    /// Next message (reply or event); `None` if nothing arrives within `timeout`.
    pub fn next_message(&mut self, timeout: Duration) -> R<Option<Value>> {
        self.ws.get_mut().set_read_timeout(Some(timeout.max(Duration::from_millis(1))))?;
        loop {
            return match self.ws.read() {
                Ok(Message::Text(t)) => Ok(Some(serde_json::from_str(t.as_str())?)),
                Ok(Message::Close(_)) => Err("browser closed the CDP connection".into()),
                Ok(_) => continue,
                Err(tungstenite::Error::Io(e))
                    if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) =>
                {
                    Ok(None)
                }
                Err(e) => Err(e.into()),
            };
        }
    }

    pub fn call_timeout(&mut self, method: &str, params: Value, session: Option<&str>, timeout: Duration) -> R<Value> {
        let id = self.send(method, params, session)?;
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(format!("timed out waiting for {method}").into());
            }
            let Some(v) = self.next_message(left)? else { continue };
            if v.get("id").and_then(Value::as_u64) != Some(id) {
                continue; // an event or another reply
            }
            if let Some(err) = v.get("error") {
                let m = err.get("message").and_then(Value::as_str).unwrap_or("CDP error");
                return Err(format!("{method}: {m}").into());
            }
            return Ok(v.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    /// Evaluate JS in a page session; awaits promises and returns the value by value.
    pub fn eval(&mut self, session: &str, expr: &str, timeout: Duration) -> R<Value> {
        let params = json!({
            "expression": expr,
            "returnByValue": true,
            "awaitPromise": true,
            "userGesture": true,
        });
        let r = self.call_timeout("Runtime.evaluate", params, Some(session), timeout)?;
        if let Some(ex) = r.get("exceptionDetails") {
            let m = ex
                .pointer("/exception/description")
                .or_else(|| ex.get("text"))
                .and_then(Value::as_str)
                .unwrap_or("JS exception");
            return Err(m.lines().next().unwrap_or(m).to_string().into());
        }
        Ok(r.pointer("/result/value").cloned().unwrap_or(Value::Null))
    }
}
