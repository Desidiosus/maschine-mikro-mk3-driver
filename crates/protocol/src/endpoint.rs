use std::path::PathBuf;

/// Resolve the driver IPC socket path: the `$MMK3_DRIVER_SOCKET` override if set,
/// else `$XDG_RUNTIME_DIR/maschine-mikro-mk3-driver/driver.sock`.
pub fn socket_path() -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var("MMK3_DRIVER_SOCKET")
        && !p.is_empty()
    {
        return Ok(PathBuf::from(p));
    }
    let runtime = std::env::var("XDG_RUNTIME_DIR")
        .map_err(|_| "XDG_RUNTIME_DIR not set; set MMK3_DRIVER_SOCKET".to_string())?;
    Ok(PathBuf::from(runtime)
        .join("maschine-mikro-mk3-driver")
        .join("driver.sock"))
}

#[cfg(test)]
mod tests {
    use super::socket_path;

    #[test]
    fn override_env_takes_precedence() {
        // SAFETY: single-threaded test; restore after.
        unsafe { std::env::set_var("MMK3_DRIVER_SOCKET", "/tmp/custom.sock") };
        assert_eq!(
            socket_path().unwrap(),
            std::path::PathBuf::from("/tmp/custom.sock")
        );
        unsafe { std::env::remove_var("MMK3_DRIVER_SOCKET") };
    }
}
