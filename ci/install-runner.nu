# Install two repository-scoped, unprivileged runners using Dyad's existing
# isolated rootless Podman service. The socket is never passed into job containers.
def main [runner: path, image: string, registration_file: path] {
  if $image !~ '^sha256:[0-9a-f]{64}$' { error make {msg: 'use the immutable local image ID'} }
  let root = $env.HOME | path join .local state fjx-runner
  let units = $env.HOME | path join .config systemd user
  mkdir $root $units
  chmod 0700 $root
  let runtime = $env.XDG_RUNTIME_DIR? | default $"/run/user/(^id -u | str trim)"
  let token = (open --raw $registration_file | from json).token
  rm --force $registration_file
  for kind in [ci release] {
    let directory = $root | path join $kind
    mkdir $directory
    chmod 0700 $directory
    let identity = $directory | path join .runner
    if not ($identity | path exists) {
      let registration = do {
        cd $directory
        ^$runner register --no-interactive --instance https://dyad.tail77425.ts.net/forgejo --token $token --name $"fjx-($kind)-dyad" --labels $"fjx-($kind):docker://($image)"
      } | complete
      if $registration.exit_code != 0 { error make {msg: $"could not register fjx-($kind); inspect Forgejo runners before retrying"} }
    }
    chmod 0600 $identity
    let timeout = if $kind == ci { '5m' } else { '45m' }
    let cpus = if $kind == ci { 4 } else { 2 }
    let config = $directory | path join config.yaml
    {
      log: {level: info}
      runner: {
        file: $identity, capacity: 1, timeout: $timeout,
        shutdown_timeout: $timeout, fetch_interval: '1s',
        labels: [$"fjx-($kind):docker://($image)"]
      }
      cache: {enabled: false}
      container: {
        docker_host: '-', privileged: false, valid_volumes: [],
        force_pull: false, force_rebuild: false,
        options: $"--cpus=($cpus) --memory=4g --memory-swap=4g --pids-limit=2048"
      }
    } | to yaml | save --force --raw $config
    [
      '[Unit]'
      $"Description=FJX ($kind) Actions runner"
      'Requires=forgejo-pr-podman.service'
      'After=forgejo-pr-podman.service'
      '[Service]'
      $"WorkingDirectory=($directory)"
      $"Environment=DOCKER_HOST=unix://($runtime)/forgejo-pr-podman/podman.sock"
      $"ExecStart=($runner) daemon --config ($config)"
      'Restart=on-failure'
      'RestartSec=2s'
      $"TimeoutStopSec=($timeout)"
      'UMask=0077'
      '[Install]'
      'WantedBy=default.target'
      ''
    ] | str join (char nl) | save --force --raw ($units | path join $"forgejo-fjx-($kind)-runner.service")
  }
  ^systemctl --user daemon-reload
  if $env.LAST_EXIT_CODE != 0 { error make {msg: 'systemd reload failed'} }
  for kind in [ci release] {
    ^systemctl --user enable $"forgejo-fjx-($kind)-runner.service"
    if $env.LAST_EXIT_CODE != 0 { error make {msg: 'runner enable failed'} }
    ^systemctl --user restart $"forgejo-fjx-($kind)-runner.service"
    if $env.LAST_EXIT_CODE != 0 { error make {msg: 'runner restart failed'} }
  }
}
