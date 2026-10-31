# Security policy

## Reporting a vulnerability

Please report vulnerabilities privately through GitHub: open the **Security**
tab of this repository and choose **Report a vulnerability** (private
vulnerability reporting), or go straight to
<https://github.com/DragonKing026/rootwhy/security/advisories/new>.
Do not open a public issue for a security problem.

## Scope

rootwhy asks for your sudo/pkexec password in a dialog, so the following count
as vulnerabilities: bypassing or spoofing the confirmation (the shown reason or
command differing from what is run), and any leak of the password (logs, state
files, environment, process arguments, other users' processes). Problems in the
plugin hook that let a command run without the dialog are in scope too.

## Supported versions

Only the latest release is supported.
