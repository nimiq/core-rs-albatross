use std::path::{Path, PathBuf};

use failure::Error;

pub struct Docker {
    directory: PathBuf,
}

impl Docker {
    pub fn new<P: AsRef<Path>>(directory: P) -> Self {
        Self {
            directory: directory.as_ref().to_path_buf(),
        }
    }

    pub fn build(&self) -> Result<(), Error> {
        info!("Building docker containers: {}", self.directory.display());
        docker_cmd::build(self.directory.to_str().unwrap())
    }

    pub fn up<'a>(&'a self) -> Result<impl Iterator<Item = Result<String, Error>> + 'a, Error> {
        info!("Starting docker containers: {}", self.directory.display());
        docker_cmd::up(self.directory.to_str().unwrap())
    }

    pub fn down(&self) -> Result<(), Error> {
        info!("Stopping docker containers: {}", self.directory.display());
        docker_cmd::down(self.directory.to_str().unwrap())
    }

    /*pub fn prune(&self) -> Result<(), Error> {
        panic!("TODO: Add filter argument to only prune nimiq devnet related containers");
        info!("Pruning docker containers: {}", self.directory.display());
        docker_cmd::prune()
    }*/
}

mod docker_cmd {
    use failure::Error;

    #[shellfn::shell]
    pub fn build<P: ToString>(env_dir: P) -> Result<(), Error> {
        r#"
        docker-compose -f $ENV_DIR/docker-compose.yml build
    "#
    }

    #[shellfn::shell]
    pub fn up<P: ToString>(env_dir: P) -> Result<impl Iterator<Item = Result<String, Error>>, Error> {
        "docker-compose -f $ENV_DIR/docker-compose.yml up | tee $ENV_DIR/build/validators.log"
    }

    #[shellfn::shell]
    pub fn down<P: ToString>(env_dir: P) -> Result<(), Error> {
        "docker-compose -f $ENV_DIR/docker-compose.yml down"
    }

    #[shellfn::shell]
    pub fn prune() -> Result<(), Error> {
        "docker image prune -f"
    }
}
