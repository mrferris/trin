use crate::portalnet::Enr;
use discv5::{
    enr::NodeId,
    kbucket::{ConnectionState, NodeStatus},
};
use serde_json::{json, Value};

pub fn bucket_entries_to_json(
    bucket_entries: Vec<(usize, Vec<(NodeId, Enr, NodeStatus)>)>,
) -> Value {
    let mut node_count: u16 = 0;
    let mut connected_count: u16 = 0;
    let buckets_indexed: Vec<(String, Vec<(String, String, String)>)> = bucket_entries
        .into_iter()
        .map(|(index, bucket)| {
            (
                format!("{}", index),
                bucket
                    .iter()
                    .map(|(node_id, enr, node_status)| {
                        node_count += 1;
                        if node_status.state == ConnectionState::Connected {
                            connected_count += 1
                        }
                        (
                            format!("0x{}", hex::encode(node_id.raw())),
                            enr.to_base64(),
                            format!("{:?}", node_status.state),
                        )
                    })
                    .collect(),
            )
        })
        .collect();

    json!(
        {
            "buckets": buckets_indexed,
            "numBuckets": buckets_indexed.len(),
            "numNodes": node_count,
            "numConnected": connected_count
        }
    )
}