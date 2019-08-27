use std::path::{Path, PathBuf};

use failure::Error;

pub struct Docker {
    directory: PathBuf,
}

impl Docker {
    pub fn new<P: AsRef<Path>>(directory: P) -> Self {
        Self {
            directory: directory.as_ref().to_path_buf()
        }
    }

    pub fn build<P: AsRef<Path>>(&self, nimiq_bin: P) -> Result<(), Error> {
        info!("Building docker containers: {}", self.directory.display());
        info!("Using nimiq-client from: {}", nimiq_bin.as_ref().display());
        docker::build(self.directory.to_str().unwrap(), nimiq_bin.as_ref().to_str().unwrap())
    }

    pub fn up<'a>(&'a self) -> Result<impl Iterator<Item=Result<String, Error>> + 'a, Error> {
        info!("Starting docker containers: {}", self.directory.display());
        docker::up(self.directory.to_str().unwrap())
    }

    pub fn down(&self) -> Result<(), Error> {
        info!("Stopping docker containers: {}", self.directory.display());
        docker::down(self.directory.to_str().unwrap())
    }

    /*pub fn prune(&self) -> Result<(), Error> {
        panic!("TODO: Add filter argument to only prune nimiq devnet related containers");
        info!("Pruning docker containers: {}", self.directory.display());
        docker::prune()
    }*/
}


mod docker {
    use failure::Error;

    #[shell]
    pub fn build<P: ToString, Q: ToString>(env_dir: P, nimiq_bin: Q) -> Result<(), Error> { r#"
        cp $NIMIQ_BIN .
        docker-compose -f $ENV_DIR/docker-compose.yml build
    "# }

    #[shell]
    pub fn up<P: ToString>(env_dir: P) -> Result<impl Iterator<Item=Result<String, Error>>, Error> {
        "docker-compose -f $ENV_DIR/docker-compose.yml up"
    }

    #[shell]
    pub fn down<P: ToString>(env_dir: P) -> Result<(), Error> {
        "docker-compose -f $ENV_DIR/docker-compose.yml down"
    }

    #[shell]
    pub fn prune() -> Result<(), Error> {
        "docker image prune -f"
    }
}
