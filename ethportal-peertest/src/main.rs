use ethportal_peertest::cli::PeertestConfig;
use log::info;
use trin_core::portalnet::{
    protocol::{PortalnetConfig, PortalnetProtocol},
    Enr, U256,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    tokio::spawn(async move {
        let peertest_config = PeertestConfig::default();
        let portal_config = PortalnetConfig {
            listen_port: peertest_config.listen_port,
            ..Default::default()
        };

        let (p2p, events) = PortalnetProtocol::new(portal_config).await.unwrap();

        tokio::spawn(events.process_discv5_requests());

        let target_enrs: Vec<Enr> = peertest_config
            .target_nodes
            .iter()
            .map(|nodestr| nodestr.parse().unwrap())
            .collect();

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

    Ok(())
}
