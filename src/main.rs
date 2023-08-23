mod device;
mod config;
mod protocol;
mod receive;
mod resample;
mod socket;
mod source;
mod stats;
mod thread;
mod time;
mod util;

use std::process::ExitCode;

use structopt::StructOpt;

#[derive(StructOpt)]
enum Opt {
    Stream(source::StreamOpt),
    Receive(receive::ReceiveOpt),
    Stats(stats::StatsOpt),
}

#[derive(Debug)]
pub enum RunError {
    Listen(socket::ListenError),
    NoDeviceAvailable,
    NoSupportedStreamConfig,
    StreamConfigs(cpal::SupportedStreamConfigsError),
    BuildStream(cpal::BuildStreamError),
    Stream(cpal::PlayStreamError),
    Socket(std::io::Error),
}

fn main() -> Result<(), ExitCode> {
    if let Some(config) = config::read() {
        config::load_into_env(&config);
    }

    let opt = Opt::from_args();

    let result = match opt {
        Opt::Stream(opt) => source::run(opt),
        Opt::Receive(opt) => receive::run(opt),
        Opt::Stats(opt) => stats::run(opt),
    };

    result.map_err(|err| {
        eprintln!("error: {err:?}");
        ExitCode::FAILURE
    })
}
