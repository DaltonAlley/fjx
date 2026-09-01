use std::process::{Command, Output};

fn run_with_host(host: &str) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_fjx"));
    command
        .args([
            "api",
            "items",
            "-X",
            "POST",
            "--dry-run",
            "--json",
            "--host",
            host,
            "-R",
            "owner/repo",
        ])
        .env("FJX_TOKEN", "test-secret")
        .env(
            "FJX_CONFIG",
            std::env::temp_dir().join(format!("fjx-host-ipv6-{}.json", std::process::id())),
        )
        .env_remove("FJX_HOST")
        .env_remove("FJX_REPO")
        .env_remove("FORGEJO_TOKEN");
    command.output().unwrap_or_else(|error| panic!("{error}"))
}

fn assert_context_error(host: &str) {
    let output = run_with_host(host);
    assert_eq!(
        output.status.code(),
        Some(3),
        "host {host:?} produced stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty(), "host {host:?} wrote stdout");
}

#[test]
fn https_ipv6_is_accepted_and_canonicalized() {
    for (host, expected) in [
        (
            "https://[2001:0DB8:0000:0000:0000:0000:0000:0001]",
            "https://[2001:db8::1]/api/v1/items",
        ),
        (
            "https://[2001:0DB8:0000:0000:0000:0000:0000:0001]:0443/Forgejo/",
            "https://[2001:db8::1]:0443/Forgejo/api/v1/items",
        ),
    ] {
        let output = run_with_host(host);
        assert!(
            output.status.success(),
            "host {host:?} produced stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!(
                "{{\"kind\":\"request\",\"method\":\"POST\",\"url\":\"{expected}\",\"body\":null}}\n"
            )
        );
    }
}

#[test]
fn plain_http_keeps_ipv6_loopback_only() {
    let output = run_with_host("http://[::1]:3000/Forgejo/");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{\"kind\":\"request\",\"method\":\"POST\",\"url\":\"http://[::1]:3000/Forgejo/api/v1/items\",\"body\":null}\n"
    );

    assert_context_error("http://[2001:db8::1]:3000");
}

#[test]
fn malformed_ipv6_and_ports_are_context_errors() {
    for host in [
        "https://[]",
        "https://[2001:db8:::1]",
        "https://[2001:db8::1",
        "https://2001:db8::1]",
        "https://[2001:db8::1]extra",
        "https://[2001:db8::1]:",
        "https://[2001:db8::1]:+443",
        "https://[2001:db8::1]:65536",
    ] {
        assert_context_error(host);
    }
}
