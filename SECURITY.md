# Security policy

## Supported versions

Only the latest release receives security fixes.

## Reporting a vulnerability

Please do **not** open a public issue for a security problem.

Use GitHub's private vulnerability reporting:

1. Open the repository's **Security** tab.
2. Choose **Report a vulnerability**.
3. Include the affected version, platform, reproduction steps, and impact.

You will get an acknowledgement as soon as possible. Once a fix is available,
a release will be published and the report will be credited unless you prefer
to remain anonymous.

## Scope notes

`portpeek` reads process metadata that may include secrets in command-line
arguments. The tool itself does not transmit data anywhere. When sharing output
or bug reports, use `--no-command` and remove sensitive paths if necessary.
