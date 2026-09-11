# The Easy Smart protocol

Reconstructed from TP-Link's JavaFX utility (decompiled) and verified against
TL-SG108PE hardware running firmware `1.0.0 Build 20201030`. The wire format is
not documented anywhere official, so this records what was learned.

## Transport

UDP. The switch listens on port **29808**, the host on **29809**.

Every request goes out as a **limited broadcast** to `255.255.255.255`,
including requests aimed at one specific switch. That is not laziness. A switch
on a foreign subnet — the factory default `192.168.0.1`, say — drops a
subnet-directed broadcast such as `10.0.255.255`, because that is not its own
broadcast address. The limited broadcast is the only way to reach it. Replies
are matched on the MAC in the header rather than on the source address.

Two consequences on Linux, both of which the vendor utility gets wrong:

- **The receiving socket must bind the wildcard address.** Switches answer to
  `255.255.255.255`, and Linux does not deliver those to a socket bound to a
  specific unicast address. The Java utility binds the interface address, which
  is why it reports *"No switch exists in the local area network!"* under a
  Linux JRE while transmitting perfectly well. Windows does deliver them, so
  the bug never showed up for the vendor.
- **Pinning the send to one interface needs `SO_BINDTODEVICE`**, not a bind to
  the interface address. It requires no privileges.

## Framing

A 32-byte header followed by big-endian TLVs, terminated by type `0xFFFF` with
length 0.

| Offset | Size | Field |
|---|---|---|
| 0 | 1 | version (always 1) |
| 1 | 1 | opcode |
| 2 | 6 | switch MAC (all zeroes = every switch) |
| 8 | 6 | host MAC |
| 14 | 2 | sequence, echoed in the reply |
| 16 | 4 | error code |
| 20 | 2 | total length, header included |
| 22 | 2 | fragment offset |
| 24 | 2 | flags |
| 26 | 2 | token |
| 28 | 4 | checksum (always zero) |

A TLV is a 2-byte type, a 2-byte length, then the value.

**Opcodes**

| Code | Meaning |
|---|---|
| 0 | discover |
| 1 | get |
| 2 | reply to discover or get |
| 3 | set |
| 4 | reply to set |

**Error codes**

| Code | Meaning |
|---|---|
| 0 | ok |
| -2 | flash write failed |
| 6 | invalid IP |
| 7 | wrong credentials |
| 8 | access denied |
| 9 | invalid netmask |
| 10 | invalid gateway |

**TLV types**

| Type | Field |
|---|---|
| 1 | model |
| 2 | description |
| 3 | MAC |
| 4 | IP |
| 5 | netmask |
| 6 | gateway |
| 7 | firmware version |
| 8 | hardware version |
| 9 | DHCP state |
| 10 | port count |
| 13 | autosave |
| 14 | factory-default flag |
| 512 | username |
| 514 | password |
| 528 | RSA session key ([a dead end](#a-dead-end-worth-recording)) |
| 2304 | save to flash |
| 2305 | request a write token ([see Writes](#writes)) |

## Encryption

The entire datagram, header included, is RC4'd with a fixed 256-byte key. This
is obfuscation, not security.

The key is not stored directly in the binary: a TEA-encrypted blob ships
instead and is decrypted at startup. `src/proto/crypto.rs` reproduces that
derivation rather than pasting the plaintext key in, so the constant stays
traceable to the vendor binary, and a test asserts the derived value.

## Reads

A get carries one empty TLV naming the field wanted; the reply carries the
populated set. Reads of ordinary fields (TLV 2, 10, …) work without
credentials, from any subnet.

Reads of *privileged* fields do not. A get of TLV 512 (username) is discarded
silently — no error, no reply.

## Writes

**Writes are gated on a single-use token.** Reading TLV 2305 makes the switch
issue a token in the reply **header**; the set that follows must carry it in
its own header. A set with token 0, or with a token already spent, is discarded
**silently** — no error code, no reply at all. This is the most confusing part
of the protocol to reverse-engineer, because a malformed packet and an
unauthorised one look identical from outside: nothing comes back.

Fetch a fresh token immediately before **every** write, the save-to-flash write
included. The vendor utility does exactly this, visible as a `GET 2305`
immediately before each `SET`.

A set carries username and password first, then the fields being changed.
String values — username, password, description — are **NUL-terminated** on the
wire. With DHCP enabled the address TLVs are sent as `0.0.0.0` rather than the
current values.

A set changes the running config only. Persisting needs a second request
carrying TLV 2304, with its own fresh token. The vendor dialog has a "save
config" checkbox for this, but its FXML marks it `visible="false"` and the
controller ticks it at startup, so in practice the utility always saves.

Descriptions must match `^[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?$` and be at
most 32 characters; the switch rejects anything else.

## A dead end worth recording

The utility also contains an RSA key exchange: TLV 528, a hand-rolled bignum
implementation with a 64-bit or 1024-bit modulus and exponent 65537, which
negotiates an alternative 8-byte stream cipher selected per-MAC.

It is unrelated to the token mechanism and is **not used** by TL-SG108PE
firmware 1.0.0 — sending it produces no reply. This tool does not implement it.
If some future switch stays silent even when given a valid token, that is the
first place to look.

## Security

Credentials travel as plain TLVs (512 and 514) inside the RC4 layer, and that
layer's key is a fixed constant extractable from the vendor binary. Anyone on
the same segment can read the switch password off the wire.

These devices are not suitable for an untrusted network, and their passwords
should not be shared with anything else.
