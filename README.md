# tplink-easysmart

A terminal UI for TP-Link "Easy Smart" switches (TL-SG105E / SG108E / SG108PE
and relatives), replacing the vendor's JavaFX utility.

It covers the two things that utility is actually needed for — finding switches
on the wire and setting their address — and hands everything else (VLANs, QoS,
port mirroring, PoE) to the switch's own web interface, which is a keypress
away.

```
cargo build --release
./target/release/tplink-easysmart
```

| Key | |
|---|---|
| `↑` `↓` / `j` `k` | select |
| `enter` / `s` | basic & IP settings |
| `w` | open the web UI in a browser |
| `r` | rescan |
| `?` | help |
| `q` | quit |

In the settings form, `tab` moves between fields, `space` toggles DHCP, and the
tab order ends on **Apply** / **Cancel**; `enter` applies from anywhere, `esc`
backs out. Applying asks for confirmation first, then writes the change and
saves it to flash.

Options: `-i/--interface <NAME>` to pick the interface (default: whichever
carries the default route), `-l/--list` to list candidates.

Switches that are not on the host's subnet — typically still on the factory
`192.168.0.1` — are shown dimmed. They are discoverable but not reachable over
IP, so give them an address first; the web UI is unavailable until then.

## Notes on the protocol

Reconstructed from the vendor utility. Useful details, since the wire format is
not documented anywhere official:

**Transport.** UDP, switch on port 29808, host on 29809. Every request goes out
as a *limited* broadcast to `255.255.255.255` — including requests aimed at one
specific switch. That is not laziness: a switch on a foreign subnet drops a
subnet-directed broadcast like `10.0.255.255` because it is not its own
broadcast address, so the limited broadcast is the only way to talk to a switch
sitting on `192.168.0.1`. Replies are matched on the MAC in the header, not on
the source address.

Two consequences on Linux, both of which the vendor utility gets wrong:

* The receiving socket must bind the **wildcard** address. Switches answer to
  `255.255.255.255`, and Linux does not deliver those to a socket bound to a
  specific unicast address. The Java utility binds the interface address, which
  is why it reports "No switch exists in the local area network!" under a Linux
  JRE while transmitting perfectly well. (Windows delivers them, hence the bug
  never showed up for the vendor.)
* Pinning the send to one interface therefore needs `SO_BINDTODEVICE` rather
  than a bind to the interface address. It needs no privileges.

**Framing.** A 32-byte header followed by big-endian TLVs, terminated by type
`0xFFFF` with length 0.

| Offset | Size | Field |
|---|---|---|
| 0 | 1 | version (always 1) |
| 1 | 1 | opcode |
| 2 | 6 | switch MAC (zeroes = broadcast to all) |
| 8 | 6 | host MAC |
| 14 | 2 | sequence, echoed in the reply |
| 16 | 4 | error code |
| 20 | 2 | total length, header included |
| 22 | 2 | fragment offset |
| 24 | 2 | flags |
| 26 | 2 | token |
| 28 | 4 | checksum (always zero) |

Opcodes: 0 discover, 1 get, 2 reply to either, 3 set, 4 reply to set.

Error codes: 0 ok, -2 flash write failed, 6 bad IP, 7 bad credentials, 8 access
denied, 9 bad netmask, 10 bad gateway.

TLV types: 1 model, 2 description, 3 MAC, 4 IP, 5 netmask, 6 gateway,
7 firmware, 8 hardware, 9 DHCP, 10 port count, 13 autosave, 14 factory-default
flag, 512 username, 514 password, 2304 save-to-flash.

**Encryption.** The entire datagram, header included, is RC4'd with a fixed
256-byte key — so this is obfuscation, not security. The key is not stored
directly in the binary: a TEA-encrypted blob is shipped and decrypted at
startup. `src/proto/crypto.rs` reproduces that derivation rather than pasting
the plaintext key in, so the constant stays traceable to the vendor binary; a
test asserts the derived value.

Credentials travel as plain TLVs (512/514) inside that RC4 layer, so anyone on
the segment can read them off the wire. Treat these switches accordingly.

**Writes.** A set request carries username and password first, then the fields.
With DHCP enabled the address TLVs are sent as `0.0.0.0`. A set changes the
running config only; persisting needs a second request carrying TLV 2304. The
vendor dialog has a "save config" checkbox for this, but its FXML marks it
`visible="false"` and the controller ticks it at startup, so in practice the
utility always saves. This tool does the same, without the dead control.
Descriptions must match `^[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?$`, 32 chars
max — the switch rejects anything else.
