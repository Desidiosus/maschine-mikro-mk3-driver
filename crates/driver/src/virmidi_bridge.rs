use std::process::Command;
use std::thread;
use std::time::Duration;

use crate::error::{DriverError, DriverResult};
use crate::settings::Settings;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeqPort {
    pub client_id: u32,
    pub port_id: u32,
    pub client_name: String,
    pub port_name: String,
}

pub fn parse_aconnect_list(output: &str) -> Vec<SeqPort> {
    let mut ports = Vec::new();
    let mut current_client: Option<(u32, String)> = None;

    for line in output.lines() {
        if let Some(rest) = line.strip_prefix("client ") {
            let Some((client_part, remainder)) = rest.split_once(':') else {
                continue;
            };
            let Ok(client_id) = client_part.trim().parse::<u32>() else {
                continue;
            };
            let Some(name_start) = remainder.find('\'') else {
                continue;
            };
            let name_slice = &remainder[name_start + 1..];
            let Some(name_end) = name_slice.find('\'') else {
                continue;
            };

            current_client = Some((client_id, name_slice[..name_end].to_string()));
            continue;
        }

        let Some((client_id, client_name)) = current_client.as_ref() else {
            continue;
        };
        let trimmed = line.trim_start();
        let Some((port_part, remainder)) = trimmed.split_once(' ') else {
            continue;
        };
        let Ok(port_id) = port_part.parse::<u32>() else {
            continue;
        };
        let Some(name_start) = remainder.find('\'') else {
            continue;
        };
        let name_slice = &remainder[name_start + 1..];
        let Some(name_end) = name_slice.find('\'') else {
            continue;
        };

        ports.push(SeqPort {
            client_id: *client_id,
            port_id,
            client_name: client_name.clone(),
            port_name: name_slice[..name_end].to_string(),
        });
    }

    ports
}

pub fn run_aconnect(from: &SeqPort, to: &SeqPort) -> DriverResult<()> {
    let status = Command::new("aconnect")
        .arg(format!("{}:{}", from.client_id, from.port_id))
        .arg(format!("{}:{}", to.client_id, to.port_id))
        .status()
        .map_err(|err| DriverError::Bridge(format!("failed to execute aconnect: {err}")))?;

    if status.success() {
        Ok(())
    } else {
        Err(DriverError::Bridge(format!(
            "aconnect exited with {status}"
        )))
    }
}

pub fn try_autoconnect_virmidi(settings: &Settings) -> DriverResult<()> {
    let mut last_err: Option<String> = None;

    for _ in 0..20 {
        let output = Command::new("aconnect")
            .arg("-l")
            .output()
            .map_err(|err| DriverError::Bridge(format!("failed to run `aconnect -l`: {err}")))?;

        if !output.status.success() {
            last_err = Some(format!("`aconnect -l` failed with {}", output.status));
            thread::sleep(Duration::from_millis(50));
            continue;
        }

        let text = String::from_utf8_lossy(&output.stdout);
        let ports = parse_aconnect_list(&text);

        let driver_out = match ports
            .iter()
            .find(|port| {
                port.client_name == settings.global.client_name
                    && port.port_name == settings.global.port_name
            })
            .cloned()
        {
            Some(port) => port,
            None => {
                last_err = Some(format!(
                    "could not find driver output port \"{}\" / \"{}\" in `aconnect -l`",
                    settings.global.client_name, settings.global.port_name
                ));
                thread::sleep(Duration::from_millis(50));
                continue;
            }
        };

        let driver_in_client = format!("{} In", settings.global.client_name);
        let driver_in = match ports
            .iter()
            .find(|port| {
                port.client_name == driver_in_client
                    && port.port_name == settings.global.port_name_in
            })
            .cloned()
        {
            Some(port) => port,
            None => {
                last_err = Some(format!(
                    "could not find driver input port \"{}\" / \"{}\" in `aconnect -l`",
                    driver_in_client, settings.global.port_name_in
                ));
                thread::sleep(Duration::from_millis(50));
                continue;
            }
        };

        let virmidi_candidates: Vec<SeqPort> =
            if settings.bridge.virmidi_client_name.trim().is_empty() {
                ports
                    .iter()
                    .filter(|port| port.client_name.starts_with("Virtual Raw MIDI"))
                    .cloned()
                    .collect()
            } else {
                ports
                    .iter()
                    .filter(|port| port.client_name == settings.bridge.virmidi_client_name)
                    .cloned()
                    .collect()
            };

        if virmidi_candidates.is_empty() {
            last_err = Some(
                "no virmidi ALSA sequencer ports found (is snd-virmidi loaded? did the host open it once?)"
                    .to_string(),
            );
            thread::sleep(Duration::from_millis(50));
            continue;
        }

        let Some(virmidi_port) = virmidi_candidates
            .into_iter()
            .find(|port| port.port_id as usize == settings.bridge.virmidi_port)
        else {
            last_err = Some(format!(
                "virmidi client found, but no port {} exists",
                settings.bridge.virmidi_port
            ));
            thread::sleep(Duration::from_millis(50));
            continue;
        };

        run_aconnect(&driver_out, &virmidi_port)?;
        run_aconnect(&virmidi_port, &driver_in)?;

        return Ok(());
    }

    Err(DriverError::Bridge(
        last_err.unwrap_or_else(|| "auto-connect failed".to_string()),
    ))
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_clients_and_ports() {
        let input = "\
client 20: 'Maschine Mikro MK3' [type=user,pid=1]\n\
    0 'Maschine Mikro MK3 MIDI Out'\n\
client 24: 'Virtual Raw MIDI 1-0' [type=kernel]\n\
    1 'VirMIDI 1-0'\n";

        let ports = crate::virmidi_bridge::parse_aconnect_list(input);

        assert_eq!(ports.len(), 2);
        assert_eq!(ports[0].client_id, 20);
        assert_eq!(ports[0].port_id, 0);
        assert_eq!(ports[0].client_name, "Maschine Mikro MK3");
        assert_eq!(ports[0].port_name, "Maschine Mikro MK3 MIDI Out");
        assert_eq!(ports[1].client_id, 24);
        assert_eq!(ports[1].port_id, 1);
        assert_eq!(ports[1].client_name, "Virtual Raw MIDI 1-0");
        assert_eq!(ports[1].port_name, "VirMIDI 1-0");
    }
}
