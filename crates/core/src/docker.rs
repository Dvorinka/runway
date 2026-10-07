//! Docker API access via bollard.
//!
//! In production the daemon is reached through a socket proxy
//! (`tcp://docker-proxy:2375`) with reduced capabilities. Locally the
//! socket is used directly.

use bollard::Docker;

use crate::config::Settings;
use crate::error::Result;

pub fn connect(settings: &Settings) -> Result<Docker> {
    let docker = if let Some(rest) = settings.docker_host.strip_prefix("unix://") {
        Docker::connect_with_unix(rest, 120, bollard::API_DEFAULT_VERSION)?
    } else {
        Docker::connect_with_http(&settings.docker_host, 120, bollard::API_DEFAULT_VERSION)?
    };
    Ok(docker)
}
