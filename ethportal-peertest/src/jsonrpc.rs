use serde_json::{to_string, Value};
use tokio::sync::mpsc;
use trin_core::portalnet::protocol::PortalnetProtocol;
use trin_core::portalnet::{
    protocol::{PortalEndpoint, PortalEndpointKind},
    Enr, U256,
};

pub struct JsonRpcHandler {
    pub p2p: PortalnetProtocol,
    pub target_enr: Enr,
    pub jsonrpc_rx: mpsc::UnboundedReceiver<PortalEndpoint>,
}

impl JsonRpcHandler {
    pub async fn process_jsonrpc_requests(mut self) {
        while let Some(cmd) = self.jsonrpc_rx.recv().await {
            use PortalEndpointKind::*;

            match cmd.kind {
                NodeInfo => {
                    let node_id = self.p2p.discovery.local_enr().to_base64();
                    let _ = cmd.resp.send(Ok(Value::String(node_id)));
                }
                RoutingTableInfo => {
                    let routing_table_info = self
                        .p2p
                        .discovery
                        .discv5
                        .table_entries_id()
                        .iter()
                        .map(|node_id| Value::String(node_id.to_string()))
                        .collect();
                    let _ = cmd.resp.send(Ok(Value::Array(routing_table_info)));
                }
                Ping => {
                    let pong = self
                        .p2p
                        .send_ping(U256::from(u64::MAX), self.target_enr.clone())
                        .await
                        .unwrap()
                        .iter()
                        .map(|result| Value::String(to_string(result).unwrap()))
                        .collect();
                    let _ = cmd.resp.send(Ok(Value::Array(pong)));
                }
                FindNodes => {
                    let nodes = self
                        .p2p
                        .send_find_nodes(vec![122; 32], self.target_enr.clone())
                        .await
                        .unwrap()
                        .iter()
                        .map(|result| Value::String(to_string(result).unwrap()))
                        .collect();
                    let _ = cmd.resp.send(Ok(Value::Array(nodes)));
                }
                FindContent => {
                    let content = self
                        .p2p
                        .send_find_content(vec![111; 32], self.target_enr.clone())
                        .await
                        .unwrap()
                        .iter()
                        .map(|result| Value::String(to_string(result).unwrap()))
                        .collect();
                    let _ = cmd.resp.send(Ok(Value::Array(content)));
                }
                // ToDo: Placeholder for DummyHistoryNetworkData and DummyStateNetworkData. We need to refactor this
                _ => {}
            }
        }
    }
}
