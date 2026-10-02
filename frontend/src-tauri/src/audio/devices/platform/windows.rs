use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait};
use log::{debug, info, warn};

use crate::audio::devices::configuration::{AudioDevice, DeviceType};

/// Configure Windows audio devices using WASAPI
pub fn configure_windows_audio(host: &cpal::Host) -> Result<Vec<AudioDevice>> {
    let mut devices = Vec::new();

    // Get WASAPI devices
    if let Ok(wasapi_host) = cpal::host_from_id(cpal::HostId::Wasapi) {
        debug!("Using WASAPI host for Windows audio device enumeration");

        // Add output devices (including loopback)
        if let Ok(output_devices) = wasapi_host.output_devices() {
            for device in output_devices {
                if let Ok(name) = device.name() {
                    // For Windows, we need to mark output devices specifically for loopback
                    // info!("Found Windows output device: {}", name);
                    devices.push(AudioDevice::new(name.clone(), DeviceType::Output));
                }
            }
        } else {
            warn!("Failed to enumerate WASAPI output devices");
        }

        // Add input devices from WASAPI
        if let Ok(input_devices) = wasapi_host.input_devices() {
            for device in input_devices {
                if let Ok(name) = device.name() {
                    // info!("Found Windows input device: {}", name);
                    devices.push(AudioDevice::new(name.clone(), DeviceType::Input));
                }
            }
        } else {
            warn!("Failed to enumerate WASAPI input devices");
        }
    } else {
        warn!("Failed to create WASAPI host, falling back to default host");
    }

    // If WASAPI failed or returned no devices, try default host as fallback
    if devices.is_empty() {
        debug!("WASAPI device enumeration failed or returned no devices, falling back to default host");
        // Add regular input devices
        if let Ok(input_devices) = host.input_devices() {
            for device in input_devices {
                if let Ok(name) = device.name() {
                    // info!("Found fallback input device: {}", name);
                    devices.push(AudioDevice::new(name.clone(), DeviceType::Input));
                }
            }
        } else {
            warn!("Failed to enumerate input devices from default host");
        }

        // Add output devices
        if let Ok(output_devices) = host.output_devices() {
            for device in output_devices {
                if let Ok(name) = device.name() {
                    // info!("Found fallback output device: {}", name);
                    devices.push(AudioDevice::new(name.clone(), DeviceType::Output));
                }
            }
        } else {
            warn!("Failed to enumerate output devices from default host");
        }
    }

    // If we still have no devices, add default devices
    if devices.is_empty() {
        warn!("No audio devices found, adding default devices only");

        // Try to add default input device
        if let Some(device) = host.default_input_device() {
            if let Ok(name) = device.name() {
                // info!("Adding default input device: {}", name);
                devices.push(AudioDevice::new(name, DeviceType::Input));
            }
        }

        // Try to add default output device
        if let Some(device) = host.default_output_device() {
            if let Ok(name) = device.name() {
                // info!("Adding default output device: {}", name);
                devices.push(AudioDevice::new(name, DeviceType::Output));
            }
        }
    }

    debug!("Found {} Windows audio devices", devices.len());
    Ok(devices)
}

/// Get Windows device and configuration using WASAPI
pub fn get_windows_device(audio_device: &AudioDevice) -> Result<(cpal::Device, cpal::SupportedStreamConfig)> {
    let wasapi_host = cpal::host_from_id(cpal::HostId::Wasapi)
        .map_err(|e| anyhow!("Failed to create WASAPI host: {}", e))?;

    // Extract the base device name without the (input) or (output) suffix
    let base_name = if audio_device.name.ends_with(" (input)") {
        audio_device.name.trim_end_matches(" (input)")
    } else if audio_device.name.ends_with(" (output)") {
        audio_device.name.trim_end_matches(" (output)")
    } else {
        &audio_device.name
    };
    let input = matches!(audio_device.device_type, DeviceType::Input);

    info!("Looking for Windows device with base name: {}", base_name);

    // Iterate all endpoints, never input_devices()/output_devices(): cpal 0.15 filters those
    // by probing every format of every endpoint (supports_input/output), which took 3-5 s per
    // device and delayed the start of recording by ~15 s. The default config is the WASAPI mix
    // format, which is what shared-mode capture and loopback use anyway (~5 ms).
    let named = wasapi_host
        .devices()?
        .filter(|d| d.name().map(|n| n == base_name || n.contains(base_name)).unwrap_or(false));
    let defaults = if input { wasapi_host.default_input_device() } else { wasapi_host.default_output_device() };
    for device in named.chain(defaults) {
        let name = device.name().unwrap_or_default();
        let config = if input { device.default_input_config() } else { device.default_output_config() };
        match config {
            Ok(config) => {
                info!("Using {} device '{}': {:?}", if input { "input" } else { "output" }, name, config);
                return Ok((device, config));
            }
            // An endpoint of the other direction with the same name.
            Err(cpal::DefaultStreamConfigError::StreamTypeNotSupported) => continue,
            Err(e) => {
                warn!("No default config for '{}' ({}); trying supported configs", name, e);
                if let Some(config) = best_supported_config(&device, input) {
                    return Ok((device, config));
                }
            }
        }
    }

    Err(anyhow!("Device not found or no compatible configuration available: {}", audio_device.name))
}

/// Slow path (probes every format): stereo F32, then any F32, then anything.
fn best_supported_config(device: &cpal::Device, input: bool) -> Option<cpal::SupportedStreamConfig> {
    let configs: Vec<cpal::SupportedStreamConfigRange> = if input {
        device.supported_input_configs().ok()?.collect()
    } else {
        device.supported_output_configs().ok()?.collect()
    };
    configs
        .iter()
        .find(|c| c.sample_format() == cpal::SampleFormat::F32 && c.channels() == 2)
        .or_else(|| configs.iter().find(|c| c.sample_format() == cpal::SampleFormat::F32))
        .or_else(|| configs.first())
        .map(|c| c.clone().with_max_sample_rate())
}
