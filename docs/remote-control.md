# Remote Control (beta)

Answer the agents already running on your Mac from your phone. Type a line in Safari and it runs
in the Clinch pane that owns that session—no Clinch account, no hosted relay, and no terminal
traffic through clinch.sh.

- **Your Mac stays the host.** Nothing executes anywhere else. The phone is a view and a keyboard
  onto the sessions that are already open, and the Mac has to be awake, online, and running Clinch.
- **Private transport.** The connection runs over your own Tailscale network. That requires
  Tailscale on both devices and a Tailscale account; Clinch itself still needs no sign-in.
- **Safari-first.** The phone client is a web app you can add to the Home Screen. There is no iOS
  app, and none is planned.
- **Approved devices only.** Pair with a single-use QR invitation that expires in five minutes and
  approve the phone on the Mac. A paired phone stays authorized until you revoke it or it goes 90
  days without connecting.

Remote Control is **off until you turn it on**. Open **Settings → Clinch → Remote Control**, or
click the phone button in the window header, for guided setup and the pairing checklist.

Beta means the protocol, pairing, and revocation model are settled and shipping in the current
release, while real-device hardening continues. Report anything that misbehaves in
[Issues](https://github.com/elliot-ylambda/clinch-terminal/issues). Full setup and threat model:
[clinch.sh/remote-control](https://clinch.sh/remote-control).
