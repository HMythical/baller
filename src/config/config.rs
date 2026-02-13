use std::{fs::{File, OpenOptions}, io::Read};

use crate::error::error::BallError;

#[derive(Debug)]
pub struct BallerConfig {
    install_dir: String
}

impl BallerConfig {
    fn default() -> Self {
        let install_dir: String;

        if cfg!(target_os = "linux") {
            install_dir = "/usr/local/bin".to_string();
        } else {
            install_dir = "C:/Program Files/Baller".to_string();
        }

        return Self {
            install_dir
        };
    }
}

impl BallerConfig {
    // parse the config file
    pub fn parse_config(baller_path: &String) -> Result<BallerConfig, BallError> {
        let mut baller_config: BallerConfig = BallerConfig::default();

        let config_path: String = format!("{}/baller.conf", baller_path);
        let mut install_dir: String = String::new();

        // open the file or create it if it doesn't already exist
        let mut config_file: File = OpenOptions::new()
            .write(true)
            .read(true)
            .create(true)
            .open(config_path)
            .map_err(|e| BallError::FileIoErr(e))?;

        let mut config_content: String = String::new();
        config_file.read_to_string(&mut config_content).map_err(|e| BallError::FileIoErr(e))?;

        // split the content of the file per line
        let entries: Vec<String> = config_content.split("\n").map(|str| str.to_string()).collect();

        for (line, entry) in entries.iter().enumerate() {
            let split: Vec<String> = entry.split(|c| c == '=' || c == ' ').map(|str| str.trim().to_string()).filter(|s| !s.is_empty()).collect();

            // if len is 0 it means the line is empty thus should be ignored
            if split.len() == 0 {
                continue;
            }

            // since the format is 'key = value' and the equal sign is lost when split, each line must have 2 elements or else it is invalid
            if split.len() != 2 {
                return Err(BallError::InvalidConfig(format!("invalid config at line[{}]: ensure all lines are in the format 'key = value'", line+1)));
            }

            match split[0].as_str() {
                "install_dir" => {
                    install_dir = split[1].replace("\"", "");
                },
                _ => { return Err(BallError::UnknownConfigEntry((line+1, split[0].clone()))); }
            }
        }

        // modify the default config if the value is provided in the config file
        if !install_dir.is_empty() {
            baller_config.install_dir = install_dir;
        }

        return Ok(baller_config);
    }
}