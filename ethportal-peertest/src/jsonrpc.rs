use serde_json::Value;
use std::sync::Arc;
use tokio::sync::mpsc;
use trin_core::portalnet::discovery::Discovery;

type Responder<T, E> = mpsc::UnboundedSender<Result<T, E>>;

#[derive(Debug)]
pub enum PortalEndpointKind {
    NodeInfo,
    RoutingTableInfo,
}

#[derive(Debug)]
pub struct PortalEndpoint {
    pub kind: PortalEndpointKind,
    pub resp: Responder<Value, String>,
}

pub struct JsonRpcHandler {
    pub discovery: Arc<Discovery>,
    pub jsonrpc_rx: mpsc::UnboundedReceiver<PortalEndpoint>,
}

impl JsonRpcHandler {
    pub async fn process_jsonrpc_requests(mut self) {
        while let Some(cmd) = self.jsonrpc_rx.recv().await {
            use PortalEndpointKind::*;

            match cmd.kind {
                NodeInfo => {
                    let node_id = self.discovery.local_enr().to_base64();
                    let _ = cmd.resp.send(Ok(Value::String(node_id)));
                }
                RoutingTableInfo => {
                    let routing_table_info = self
                        .discovery
                        .discv5
                        .table_entries_id()
                        .iter()
                        .map(|node_id| Value::String(node_id.to_string()))
                        .collect();
                    let _ = cmd.resp.send(Ok(Value::Array(routing_table_info)));
                }
            }
        }
    }
}
