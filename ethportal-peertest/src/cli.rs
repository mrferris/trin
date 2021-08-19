use std::env;
use std::ffi::OsString;
use structopt::StructOpt;

const DEFAULT_LISTEN_PORT: &str = "9876";

#[derive(StructOpt, Debug, PartialEq)]
#[structopt(
    name = "trin-devp2p2",
    version = "0.0.1",
    about = "Testing framework for portal network peer-to-peer network calls"
)]
pub struct PeertestConfig {
    #[structopt(
        default_value(DEFAULT_LISTEN_PORT),
        short = "p",
        long = "listen_port",
        help = "The UDP port to listen on."
    )]
    pub listen_port: u16,

    #[structopt(
        use_delimiter = true,
        short = "tn",
        long = "target_node",
        help = "Base64-encoded ENR's of the nodes under test"
    )]
    pub target_nodes: Vec<String>,
}

impl PeertestConfig {
    pub fn new() -> Self {
        Self::new_from(env::args_os()).expect("Could not parse trin arguments")
    }

    pub fn new_from<I, T>(args: I) -> Result<Self, String>
    where
        I: Iterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let config = Self::from_iter(args);

        Ok(config)
    }
}

impl Default for PeertestConfig {
    fn default() -> Self {
        Self::new()
    }
}
