use ethportal_peertest::{cli::PeertestConfig, jsonrpc::JsonRpcHandler};
use log::info;
use threadpool::ThreadPool;
use tokio::sync::mpsc;
use trin_core::{
    jsonrpc::launch_ipc_client,
    portalnet::{
        protocol::{PortalEndpoint, PortalnetConfig, PortalnetProtocol},
        Enr, U256,
    },
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let peertest_config = PeertestConfig::default();

    let pool = ThreadPool::new(2_usize);
    let (jsonrpc_tx, jsonrpc_rx) = mpsc::unbounded_channel::<PortalEndpoint>();

    let portal_config = PortalnetConfig {
        listen_port: peertest_config.listen_port,
        ..Default::default()
    };

    let target_enrs: Vec<Enr> = peertest_config
        .target_nodes
        .iter()
        .map(|nodestr| nodestr.parse().unwrap())
        .collect();

    let web3_server_task = tokio::task::spawn_blocking(move || {
        launch_ipc_client(
            pool,
            None,
            peertest_config.web3_ipc_path.as_str(),
            jsonrpc_tx,
        );
    });

    tokio::spawn(async move {
        let (p2p, events) = PortalnetProtocol::new(portal_config).await.unwrap();

        let rpc_handler = JsonRpcHandler {
            p2p: p2p.clone(),
            target_enr: target_enrs[0].clone(),
            jsonrpc_rx,
        };

        tokio::spawn(events.process_discv5_requests());
        tokio::spawn(rpc_handler.process_jsonrpc_requests());

        // Test Pong, Node and FoundContent on target nodes
        for enr in &target_enrs {
            info!("Pinging {} on portal network", enr);
            let ping_result = p2p
                .send_ping(U256::from(u64::MAX), enr.clone())
                .await
                .unwrap();
            info!("Portal network Ping result: {:?}", ping_result);

            info!("Sending FindNodes to {}", enr);
            let nodes_result = p2p
                .send_find_nodes(vec![122; 32], enr.clone())
                .await
                .unwrap();
            info!("Got Nodes result: {:?}", nodes_result);

            info!("Sending FindContent to {}", enr);
            let content_result = p2p
                .send_find_content(vec![100; 32], enr.clone())
                .await
                .unwrap();
            info!("Got FoundContent result: {:?}", content_result);
        }

        tokio::signal::ctrl_c()
            .await
            .expect("failed to pause until ctrl-c");
    })
    .await
    .unwrap();

    web3_server_task.await.unwrap();

    Ok(())
}
