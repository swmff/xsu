use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    env, fs,
    io::{Error, ErrorKind, Result},
    process::{Child, Command},
};
use sysinfo::{Pid, System};

pub type ServiceStates = HashMap<String, (ServiceState, u32)>;

/// A single executable service
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Service {
    /// What command is run to start the service
    pub command: String,
    /// Where the `command` is run
    pub working_directory: String,
    /// Environment variables map
    pub environment: Option<HashMap<String, String>>,
    /// If the service should restart automatically when exited (HTTP server required)
    #[serde(default)]
    pub restart: bool,
}

impl Service {
    /// Spawn service process
    pub fn run(name: String, cnf: ServicesConfiguration) -> Result<Child> {
        // check current state
        if let Some(s) = cnf.service_states.get(&name) {
            // make sure service isn't already running
            if s.0 == ServiceState::Running {
                return Err(Error::new(
                    ErrorKind::AlreadyExists,
                    "Service is already running.",
                ));
            }
        };

        let service = match cnf.services.get(&name) {
            Some(s) => s,
            None => return Err(Error::new(ErrorKind::NotFound, "Service does not exist.")),
        };

        // create command
        let command_split: Vec<&str> = service.command.split(" ").collect();
        let mut cmd = Command::new(command_split.get(0).unwrap());

        for arg in command_split.iter().skip(1) {
            cmd.arg(arg);
        }

        if let Some(env) = service.environment.clone() {
            for var in env {
                cmd.env(var.0, var.1);
            }
        }

        cmd.current_dir(service.working_directory.clone());

        // spawn
        Ok(cmd.spawn()?)
    }

    /// Kill service process
    pub fn kill(name: String, config: ServicesConfiguration) -> Result<()> {
        let s = match config.service_states.get(&name) {
            Some(s) => s,
            None => return Err(Error::new(ErrorKind::NotFound, "Service is not loaded.")),
        };

        if s.0 != ServiceState::Running {
            return Err(Error::new(
                ErrorKind::NotConnected,
                "Service is not running.",
            ));
        }

        let mut config_c = config.clone();
        let service = match config_c.services.get_mut(&name) {
            Some(s) => s,
            None => return Err(Error::new(ErrorKind::NotFound, "Service does not exist.")),
        };

        // stop service
        let sys = System::new_all();

        match sys.process(Pid::from(s.1 as usize)) {
            Some(process) => {
                let supposed_to_restart = service.restart.clone();

                // if service is supposed to restart, toggle off and update config
                if supposed_to_restart {
                    // we must do this so threads that will restart this service don't
                    service.restart = false;
                    ServicesConfiguration::update_config(config_c.clone())?;
                }

                // kill process
                process.kill();
                std::thread::sleep(std::time::Duration::from_secs(1)); // wait for 1s so the server can catch up

                // if service was previously supposed to restart, re-enable restart
                if supposed_to_restart {
                    // set config back to original form
                    ServicesConfiguration::update_config(config.clone())?;
                }

                // return
                Ok(())
            }
            None => Err(Error::new(
                ErrorKind::NotConnected,
                "Failed to get process from PID.",
            )),
        }
    }

    /// Get service process info
    pub fn info(name: String, service_states: ServiceStates) -> Result<String> {
        let s = match service_states.get(&name) {
            Some(s) => s,
            None => return Err(Error::new(ErrorKind::NotFound, "Service is not loaded.")),
        };

        if s.0 != ServiceState::Running {
            return Err(Error::new(
                ErrorKind::NotConnected,
                "Service is not running.",
            ));
        }

        // get service info
        let sys = System::new_all();

        if let Some(process) = sys.process(Pid::from(s.1 as usize)) {
            let info = ServiceInfo {
                name: name.to_string(),
                pid: process.pid().to_string().parse().unwrap(),
                memory: process.memory(),
                cpu: process.cpu_usage(),
                status: process.status().to_string(),
                running_for_seconds: process.run_time(),
            };

            Ok(toml::to_string_pretty(&info).unwrap())
        } else {
            Err(Error::new(
                ErrorKind::NotConnected,
                "Failed to get process from PID.",
            ))
        }
    }

    // exit handling

    /// Wait for a service process to stop and update its state when it does
    pub async fn observe(name: String, service_states: ServiceStates) -> Result<()> {
        let s = match service_states.get(&name) {
            Some(s) => s,
            None => return Err(Error::new(ErrorKind::NotFound, "Service is not loaded.")),
        };

        if s.0 != ServiceState::Running {
            return Err(Error::new(
                ErrorKind::NotConnected,
                "Service is not running.",
            ));
        }

        // get service
        let sys = System::new_all();

        if let Some(process) = sys.process(Pid::from(s.1 as usize)) {
            // wait for process to stop
            process.wait();
            Ok(())
        } else {
            Err(Error::new(
                ErrorKind::NotConnected,
                "Failed to get process from PID.",
            ))
        }
    }

    /// Start and observe a service
    async fn wait(name: String, config: &mut ServicesConfiguration) -> Result<()> {
        // start service
        let process = match Service::run(name.clone(), config.clone()) {
            Ok(p) => p,
            Err(e) => return Err(e),
        };

        // update config
        config
            .service_states
            .insert(name.to_string(), (ServiceState::Running, process.id()));

        ServicesConfiguration::update_config(config.clone()).expect("Failed to update config");
        Service::observe(name.clone(), config.service_states.clone())
            .await
            .expect("Failed to observe service");

        Ok(())
    }

    /// [`wait`] in a new task
    pub async fn spawn(name: String) -> Result<()> {
        // spawn task
        tokio::task::spawn(async move {
            loop {
                // pull config from file
                let mut config = ServicesConfiguration::get_config();

                // start service
                Service::wait(name.clone(), &mut config)
                    .await
                    .expect("Failed to wait for service");

                // pull real config
                // we have to do this so we don't restart if it was disabled while the service was running
                let mut config = ServicesConfiguration::get_config();
                let service = match config.services.get(&name) {
                    Some(s) => s,
                    None => return,
                };

                // update config
                config.service_states.remove(&name);
                ServicesConfiguration::update_config(config.clone())
                    .expect("Failed to update config");

                // ...
                if service.restart == false {
                    // no need to loop again if we aren't supposed to restart the service
                    break;
                }

                // begin restart
                println!("info: auto-restarting service \"{}\"", name);
                continue; // service will be run again
            }
        });

        // return
        Ok(())
    }
}

/// The state of a [`Service`]
#[derive(Serialize, Deserialize, PartialEq, Clone, Debug)]
pub enum ServiceState {
    Running,
    Stopped,
}

impl Default for ServiceState {
    fn default() -> Self {
        Self::Stopped
    }
}

/// General information about a [`ServiceState`]
#[derive(Serialize, Deserialize)]
pub struct ServiceInfo {
    pub name: String,
    pub pid: u32,
    pub memory: u64,
    pub cpu: f32,
    pub status: String,
    pub running_for_seconds: u64,
}

/// `server` key in [`ServicesConfiguration`]
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ServerConfiguration {
    /// The port to serve the HTTP server on (6374 by default)
    pub port: u16,
    /// The key that is required to run operations from the HTTP server
    pub key: String,
}

impl Default for ServerConfiguration {
    fn default() -> Self {
        Self {
            port: 6374,
            key: String::new(),
        }
    }
}

/// `services.toml` file
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ServicesConfiguration {
    /// Inherited service definition files
    pub inherit: Option<Vec<String>>,
    /// Service definitions
    pub services: HashMap<String, Service>,
    /// Server configuration (`sproc serve`)
    #[serde(default)]
    pub server: ServerConfiguration,
    /// Service states
    #[serde(default)]
    pub service_states: ServiceStates,
}

impl Default for ServicesConfiguration {
    fn default() -> Self {
        Self {
            inherit: None,
            services: HashMap::new(),
            server: ServerConfiguration::default(),
            service_states: HashMap::new(),
        }
    }
}
impl ServicesConfiguration {
    pub fn get_config() -> ServicesConfiguration {
        let home = env::var("HOME").expect("failed to read $HOME");

        if let Err(_) = fs::read_dir(format!("{home}/.config/sproc")) {
            if let Err(e) = fs::create_dir(format!("{home}/.config/sproc")) {
                panic!("{:?}", e)
            };
        }

        match fs::read_to_string(format!("{home}/.config/sproc/services.toml")) {
            Ok(c) => {
                let mut res = toml::from_str::<Self>(&c).unwrap();

                // handle inherits
                if let Some(ref inherit) = res.inherit {
                    for path in inherit {
                        if let Ok(c) = fs::read_to_string(path) {
                            for service in toml::from_str::<Self>(&c).unwrap().services {
                                // push service to main service stack
                                res.services.insert(service.0, service.1);
                            }
                        }
                    }
                }

                // return
                res
            }
            Err(_) => Self::default(),
        }
    }

    pub fn update_config(contents: Self) -> std::io::Result<()> {
        let home = env::var("HOME").expect("failed to read $HOME");

        fs::write(
            format!("{home}/.config/sproc/services.toml"),
            format!("# DO **NOT** MANUALLY EDIT THIS FILE! Please edit the source instead and run `sproc pin {{path}}`.\n{}", toml::to_string_pretty::<Self>(&contents).unwrap()),
        )
    }
}
