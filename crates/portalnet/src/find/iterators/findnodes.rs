// Copyright 2018 Parity Technologies (UK) Ltd.
//
// Permission is hereby granted, free of charge, to any person obtaining a
// copy of this software and associated documentation files (the "Software"),
// to deal in the Software without restriction, including without limitation
// the rights to use, copy, modify, merge, publish, distribute, sublicense,
// and/or sell copies of the Software, and to permit persons to whom the
// Software is furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
// OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
// FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
// DEALINGS IN THE SOFTWARE.

// This basis of this file has been taken from the rust-libp2p codebase:
// https://github.com/libp2p/rust-libp2p

use std::{
    collections::btree_map::{BTreeMap, Entry},
    time::Instant,
};

use discv5::kbucket::{Distance, Key};

use super::{
    super::query_pool::QueryState,
    query::{Query, QueryConfig, QueryPeer, QueryPeerState, QueryProgress},
};

#[derive(Debug, Clone)]
pub struct FindNodeQuery<TNodeId> {
    /// The target key we are looking for
    target_key: Key<TNodeId>,

    /// The instant when the query started.
    started: Option<Instant>,

    /// The current state of progress of the query.
    progress: QueryProgress,

    /// The closest peers to the target, ordered by increasing distance.
    /// Assumes that no two peers are equidistant.
    closest_peers: BTreeMap<Distance, QueryPeer<TNodeId>>,

    /// The number of peers for which the query is currently waiting for results.
    num_waiting: usize,

    /// The configuration of the query.
    config: QueryConfig,
}

impl<TNodeId> Query<TNodeId> for FindNodeQuery<TNodeId>
where
    TNodeId: Into<Key<TNodeId>> + Eq + Clone,
{
    type Response = Vec<TNodeId>;
    type Result = Vec<TNodeId>;
    type PendingValidationResult = ();

    fn config(&self) -> &QueryConfig {
        &self.config
    }

    fn target(&self) -> Key<TNodeId> {
        self.target_key.clone()
    }

    fn started(&self) -> Option<Instant> {
        self.started
    }

    fn start(&mut self, start: Instant) {
        self.started = Some(start);
    }

    fn on_failure(&mut self, peer: &TNodeId) {
        if let QueryProgress::Finished = self.progress {
            return;
        }

        let key: Key<TNodeId> = peer.clone().into();
        let distance = key.distance(&self.target_key);

        match self.closest_peers.entry(distance) {
            Entry::Vacant(_) => {}
            Entry::Occupied(mut e) => match e.get().state() {
                QueryPeerState::Waiting(..) => {
                    debug_assert!(self.num_waiting > 0);
                    self.num_waiting -= 1;
                    e.get_mut().set_state(QueryPeerState::Failed);
                }
                QueryPeerState::Unresponsive => e.get_mut().set_state(QueryPeerState::Failed),
                _ => {}
            },
        }
    }

    /// Delivering results of requests back to the query allows the query to make
    /// progress. The query is said to make progress either when the given
    /// `peer_response` contain a peer closer to the target than any peer seen so far,
    /// or when the query did not yet accumulate `num_results` closest peers and
    /// `peer_response` contains a new peer, regardless of its distance to the target.
    fn on_success(&mut self, peer: &TNodeId, peer_response: Self::Response) {
        if let QueryProgress::Finished = self.progress {
            return;
        }

        let key: Key<TNodeId> = peer.clone().into();
        let distance = key.distance(&self.target_key);

        // Mark the peer's progress, the total nodes it has returned and it's current iteration.
        // If the node returned peers, mark it as succeeded.
        match self.closest_peers.entry(distance) {
            Entry::Vacant(..) => return,
            Entry::Occupied(mut entry) => match entry.get().state() {
                QueryPeerState::Waiting(..) => {
                    assert!(
                        self.num_waiting > 0,
                        "Query has invalid number of waiting queries"
                    );
                    self.num_waiting -= 1;
                    let peer = entry.get_mut();
                    peer.increment_peers_returned(peer_response.len());
                    peer.set_state(QueryPeerState::Succeeded);
                }
                QueryPeerState::Unresponsive => {
                    let peer = entry.get_mut();
                    peer.increment_peers_returned(peer_response.len());
                    peer.set_state(QueryPeerState::Succeeded);
                }
                QueryPeerState::NotContacted
                | QueryPeerState::Failed
                | QueryPeerState::Succeeded => return,
            },
        }

        let mut progress = false;
        let num_closest = self.closest_peers.len();

        // Incorporate the reported closer peers into the query.
        for peer in peer_response {
            let key: Key<TNodeId> = peer.into();
            let distance = self.target_key.distance(&key);
            let peer = QueryPeer::new(key, QueryPeerState::NotContacted);
            self.closest_peers.entry(distance).or_insert(peer);
            // The query makes progress if the new peer is either closer to the target
            // than any peer seen so far (i.e. is the first entry), or the query did
            // not yet accumulate enough closest peers.
            progress = self.closest_peers.keys().next() == Some(&distance)
                || num_closest < self.config.num_results;
        }

        // Update the query progress.
        self.progress = match self.progress {
            QueryProgress::Iterating { no_progress } => {
                let no_progress = if progress { 0 } else { no_progress + 1 };
                if no_progress >= self.config.parallelism {
                    QueryProgress::Stalled
                } else {
                    QueryProgress::Iterating { no_progress }
                }
            }
            QueryProgress::Stalled => {
                if progress {
                    QueryProgress::Iterating { no_progress: 0 }
                } else {
                    QueryProgress::Stalled
                }
            }
            QueryProgress::Finished => QueryProgress::Finished,
        }
    }

    fn poll(&mut self, now: Instant) -> QueryState<TNodeId> {
        if let QueryProgress::Finished = self.progress {
            return QueryState::Finished;
        }

        // Count the number of peers that returned a result. If there is a
        // request in progress to one of the `num_results` closest peers, the
        // counter is set to `None` as the query can only finish once
        // `num_results` closest peers have responded (or there are no more
        // peers to contact, see `active_counter`).
        let mut result_counter = Some(0);

        // Check if the query is at capacity w.r.t. the allowed parallelism.
        let at_capacity = self.at_capacity();

        for peer in self.closest_peers.values_mut() {
            match peer.state() {
                QueryPeerState::NotContacted => {
                    // This peer is waiting to be reiterated.
                    if !at_capacity {
                        let timeout = now + self.config.peer_timeout;
                        peer.set_state(QueryPeerState::Waiting(timeout));
                        self.num_waiting += 1;
                        let peer = peer.key().preimage().clone();
                        return QueryState::Waiting(Some(peer));
                    } else {
                        return QueryState::WaitingAtCapacity;
                    }
                }
                QueryPeerState::Waiting(timeout) => {
                    if now >= *timeout {
                        // Peers that don't respond within timeout are set to `Failed`.
                        assert!(
                            self.num_waiting > 0,
                            "Query reached invalid number of waiting queries"
                        );
                        self.num_waiting -= 1;
                        peer.set_state(QueryPeerState::Unresponsive);
                    } else if at_capacity {
                        // The query is still waiting for a result from a peer and is
                        // at capacity w.r.t. the maximum number of peers being waited on.
                        return QueryState::WaitingAtCapacity;
                    } else {
                        // The query is still waiting for a result from a peer and the
                        // `result_counter` did not yet reach `num_results`. Therefore
                        // the query is not yet done, regardless of already successful
                        // queries to peers farther from the target.
                        result_counter = None;
                    }
                }
                QueryPeerState::Succeeded => {
                    if let Some(ref mut count) = result_counter {
                        *count += 1;
                        // If `num_results` successful results have been delivered for the
                        // closest peers, the query is done.
                        if *count >= self.config.num_results {
                            self.progress = QueryProgress::Finished;
                            return QueryState::Finished;
                        }
                    }
                }
                QueryPeerState::Failed | QueryPeerState::Unresponsive => {
                    // Skip over unresponsive or failed peers.
                }
            }
        }

        if self.num_waiting > 0 {
            // The query is still waiting for results and not at capacity w.r.t.
            // the allowed parallelism, but there are no new peers to contact
            // at the moment.
            QueryState::Waiting(None)
        } else {
            // The query is finished because all available peers have been contacted
            // and the query is not waiting for any more results.
            self.progress = QueryProgress::Finished;
            QueryState::Finished
        }
    }

    fn into_result(self) -> Self::Result {
        self.closest_peers
            .into_iter()
            .filter_map(|(_, peer)| {
                if let QueryPeerState::Succeeded = peer.state() {
                    Some(peer.key().clone().into_preimage())
                } else {
                    None
                }
            })
            .take(self.config.num_results)
            .collect()
    }

    fn pending_validation_result(&self, _sending_peer: TNodeId) -> Self::PendingValidationResult {
        // FindNodes queries don't need to provide access to the result until the query is finished.
        // This is because the query doesn't run content validation, so as soon as the nodes are
        // returned, the query is considered finished.
        unimplemented!("Use `into_result` instead. FindNodes does not run content validation.")
    }
}

impl<TNodeId> FindNodeQuery<TNodeId>
where
    TNodeId: Into<Key<TNodeId>> + Eq + Clone,
{
    /// Creates a new query with the given configuration.
    pub fn with_config<I>(
        config: QueryConfig,
        target_key: Key<TNodeId>,
        known_closest_peers: I,
    ) -> Self
    where
        I: IntoIterator<Item = Key<TNodeId>>,
    {
        // Initialise the closest peers to begin the query with.
        let closest_peers = known_closest_peers
            .into_iter()
            .map(|key| {
                let key: Key<TNodeId> = key;
                let distance = key.distance(&target_key);
                let state = QueryPeerState::NotContacted;
                (distance, QueryPeer::new(key, state))
            })
            .take(config.num_results)
            .collect();

        // The query initially makes progress by iterating towards the target.
        let progress = QueryProgress::Iterating { no_progress: 0 };

        FindNodeQuery {
            config,
            target_key,
            started: None,
            progress,
            closest_peers,
            num_waiting: 0,
        }
    }

    /// Checks if the query is at capacity w.r.t. the permitted parallelism.
    ///
    /// While the query is stalled, up to `num_results` parallel requests
    /// are allowed. This is a slightly more permissive variant of the
    /// requirement that the initiator "resends the FIND_NODE to all of the
    /// k closest nodes it has not already queried".
    fn at_capacity(&self) -> bool {
        match self.progress {
            QueryProgress::Stalled => self.num_waiting >= self.config.num_results,
            QueryProgress::Iterating { .. } => self.num_waiting >= self.config.parallelism,
            QueryProgress::Finished => true,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::time::Duration;

    use discv5::enr::NodeId;
    use quickcheck::*;
    use rand::{rng, Rng};
    use test_log::test;

    use super::*;

    type TestQuery = FindNodeQuery<NodeId>;

    fn random_nodes(n: usize) -> impl Iterator<Item = NodeId> + Clone {
        (0..n).map(|_| NodeId::random())
    }

    fn random_query() -> TestQuery {
        let mut rng = rng();

        let known_closest_peers = random_nodes(rng.random_range(1..60)).map(Key::from);
        let target = NodeId::random();
        let config = QueryConfig {
            parallelism: rng.random_range(1..10),
            num_results: rng.random_range(1..25),
            peer_timeout: Duration::from_secs(rng.random_range(10..30)),
            overall_timeout: Duration::from_secs(rng.random_range(30..120)),
        };
        FindNodeQuery::with_config(config, target.into(), known_closest_peers)
    }

    fn sorted(target: &Key<NodeId>, peers: &[Key<NodeId>]) -> bool {
        peers
            .windows(2)
            .all(|w| w[0].distance(target) < w[1].distance(target))
    }

    impl Arbitrary for TestQuery {
        fn arbitrary(_: &mut Gen) -> TestQuery {
            random_query()
        }
    }

    #[test]
    fn new_query() {
        let query = random_query();
        let target = query.target_key.clone();

        let (keys, states): (Vec<_>, Vec<_>) = query
            .closest_peers
            .values()
            .map(|e| (e.key().clone(), e.state()))
            .unzip();

        let none_contacted = states
            .iter()
            .all(|s| matches!(s, QueryPeerState::NotContacted));

        assert!(none_contacted, "Unexpected peer state in new query.");
        assert!(
            sorted(&target, &keys),
            "Closest peers in new query not sorted by distance to target."
        );
        assert_eq!(
            query.num_waiting, 0,
            "Unexpected peers in progress in new query."
        );
        assert!(
            query.into_result().is_empty(),
            "Unexpected closest peers in new query"
        );
    }

    #[test]
    fn termination_and_parallelism() {
        fn prop(mut query: TestQuery) {
            let now = Instant::now();
            let mut rng = rng();

            let mut expected = query
                .closest_peers
                .values()
                .map(|e| e.key().clone())
                .collect::<Vec<_>>();
            let num_known = expected.len();
            let max_parallelism = usize::min(query.config.parallelism, num_known);

            let target = query.target_key.clone();
            let mut remaining;
            let mut num_failures = 0;

            'finished: loop {
                if expected.is_empty() {
                    break;
                }
                // Split off the next up to `parallelism` expected peers.
                else if expected.len() < max_parallelism {
                    remaining = Vec::new();
                } else {
                    remaining = expected.split_off(max_parallelism);
                }

                // Advance the query for maximum parallelism.
                for k in expected.iter() {
                    match query.poll(now) {
                        QueryState::Finished => break 'finished,
                        QueryState::Waiting(Some(p)) => assert_eq!(&p, k.preimage()),
                        QueryState::Waiting(None) => panic!("Expected another peer."),
                        QueryState::WaitingAtCapacity => panic!("Unexpectedly reached capacity."),
                        QueryState::Validating(..) => panic!("Should not validate nodes."),
                    }
                }
                let num_waiting = query.num_waiting;
                assert_eq!(num_waiting, expected.len());

                // Check the bounded parallelism.
                if query.at_capacity() {
                    assert_eq!(query.poll(now), QueryState::WaitingAtCapacity)
                }

                // Report results back to the query with a random number of "closer"
                // peers or an error, thus finishing the "in-flight requests".
                for (i, k) in expected.iter().enumerate() {
                    if rng.random_bool(0.75) {
                        let num_closer = rng.random_range(0..query.config.num_results + 1);
                        let closer_peers = random_nodes(num_closer).collect::<Vec<_>>();
                        remaining.extend(closer_peers.iter().map(|x| Key::from(*x)));
                        query.on_success(k.preimage(), closer_peers);
                    } else {
                        num_failures += 1;
                        query.on_failure(k.preimage());
                    }
                    assert_eq!(query.num_waiting, num_waiting - (i + 1));
                }

                // Re-sort the remaining expected peers for the next "round".
                remaining.sort_by_key(|k| target.distance(k));

                expected = remaining;
            }

            // The query must be finished.
            assert_eq!(query.poll(now), QueryState::Finished);
            assert_eq!(query.progress, QueryProgress::Finished);

            // Determine if all peers have been contacted by the query. This _must_ be
            // the case if the query finished with fewer than the requested number
            // of results.
            let all_contacted = query.closest_peers.values().all(|e| {
                !matches!(
                    e.state(),
                    QueryPeerState::NotContacted | QueryPeerState::Waiting { .. }
                )
            });

            let target_key = query.target_key.clone();
            let num_results = query.config.num_results;
            let result = query.into_result();
            let closest = result.into_iter().map(Key::from).collect::<Vec<_>>();

            assert!(sorted(&target_key, &closest));

            if closest.len() < num_results {
                // The query returned fewer results than requested. Therefore
                // either the initial number of known peers must have been
                // less than the desired number of results, or there must
                // have been failures.
                assert!(num_known < num_results || num_failures > 0);
                // All peers must have been contacted.
                assert!(all_contacted, "Not all peers have been contacted.");
            } else {
                assert_eq!(num_results, closest.len(), "Too  many results.");
            }
        }

        QuickCheck::new().tests(10).quickcheck(prop as fn(_) -> _)
    }

    #[test]
    fn no_duplicates() {
        fn prop(mut query: TestQuery) -> bool {
            let now = Instant::now();
            let closer: Vec<NodeId> = random_nodes(1).collect();

            // A first peer reports a "closer" peer.
            let peer1 = if let QueryState::Waiting(Some(p)) = query.poll(now) {
                p
            } else {
                panic!("No peer.");
            };
            query.on_success(&peer1, closer.clone());
            // Duplicate result from the same peer.
            query.on_success(&peer1, closer.clone());

            // If there is a second peer, let it also report the same "closer" peer.
            match query.poll(now) {
                QueryState::Waiting(Some(p)) => {
                    let peer2 = p;
                    query.on_success(&peer2, closer.clone())
                }
                QueryState::Finished => {}
                _ => panic!("Unexpectedly query state."),
            };

            // The "closer" peer must only be in the query once.
            let n = query
                .closest_peers
                .values()
                .filter(|e| e.key().preimage() == &closer[0])
                .count();
            assert_eq!(n, 1);

            true
        }

        QuickCheck::new().tests(10).quickcheck(prop as fn(_) -> _)
    }

    #[test]
    fn timeout() {
        fn prop(mut query: TestQuery) -> bool {
            let mut now = Instant::now();
            let peer = query
                .closest_peers
                .values()
                .next()
                .unwrap()
                .key()
                .clone()
                .into_preimage();
            // Poll the query for the first peer to be in progress.
            match query.poll(now) {
                QueryState::Waiting(Some(id)) => assert_eq!(id, peer),
                _ => panic!(),
            }

            // Artificially advance the clock.
            now += query.config.peer_timeout;

            // Advancing the query again should mark the first peer as unresponsive.
            let _ = query.poll(now);
            let first_peer = &query.closest_peers.values().next().unwrap();
            match first_peer.state() {
                QueryPeerState::Unresponsive => {
                    assert_eq!(first_peer.key().preimage(), &peer);
                }
                _ => panic!("Unexpected peer state: {:?}", first_peer.state()),
            }

            let finished = query.progress == QueryProgress::Finished;
            query.on_success(&peer, Vec::<NodeId>::new());
            let closest = query.into_result();

            if finished {
                // Delivering results when the query already finished must have
                // no effect.
                assert_eq!(Vec::<NodeId>::new(), closest);
            } else {
                // Unresponsive peers can still deliver results while the iterator
                // is not finished.
                assert_eq!(vec![peer], closest)
            }
            true
        }

        QuickCheck::new().tests(10).quickcheck(prop as fn(_) -> _)
    }
}
