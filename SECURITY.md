# Security policy

## Reporting a vulnerability

Do not disclose suspected vulnerabilities in public issues or pull requests.
Never include real tokens, authorization headers, private repository data, or
the contents of your token store in a report.

Use the repository's **Security > Advisories > Report a vulnerability** option
when it is available. Private reporting must be enabled by the repository owner.
If the option is unavailable, open an issue containing only a request for a
private reporting channel, without vulnerability details. Wait for the maintainer
to establish that channel before sharing a reproducer.

A useful private report includes:

- The affected fjx version or commit, operating system, and Forgejo version.
- Expected behavior, actual behavior, and the security impact.
- Minimal reproduction steps using a disposable account and fake credentials.
- Any proposed fix or regression test, if available.

There is no guaranteed response time or paid support agreement. Coordinate
disclosure with the maintainer rather than publishing exploit details first.

## Scope and fixes

Security fixes are developed on `main`. This project does not maintain separate
long-term-support branches or promise backports to older versions. Check the
[changelog](CHANGELOG.md) and repository advisories before upgrading.

Relevant boundaries include credential handling and redaction, host and raw API
path validation, private config-file access, redirect rejection, safe writes,
and output that could expose secrets or mislead scripts. See the
[architecture guide](docs/architecture.md) for the modules responsible for them.

Report vulnerabilities in a Forgejo server or another dependency through that
project's own security process, unless fjx's use of it is part of the problem.
