# Fortune

Small program implementing the [RFC 865 - Quote of the Day](https://www.rfc-editor.org/rfc/rfc865).

This is a dummy implementation which can:
- generate a quote
- start a TCP and an UDP server to deliver quotes

> Quotes have been copy-pasted from the Internet.

CLI usage is shown using `--help`

```
Implements a fortune cookie / quote of the day API (RFC 865).

Usage: fortune [OPTIONS] <COMMAND>

Commands:
  server    Start a Quote Of The Day TCP and UDP server (RFC 865)
  generate  Prints a Quote Of The Day message
  help      Print this message or the help of the given subcommand(s)

Options:
      --seed <SEED>  Seed value to initialize randomizer
  -h, --help         Print help
  -V, --version      Print version
```

## Generate

```
Start a Quote Of The Day TCP and UDP server (RFC 865)

- TCP server: for each new TCP connection, it sends a quote
- UDP server: for each datagram received, it sends a quote

Both servers ignore the content of the packet, they blindly send quotes.


Usage: fortune server [OPTIONS]

Options:
  -t, --listen-tcp <TCP_ADDRESS>  Bind server to given TCP address [default: 127.0.0.1:8080]
  -u, --listen-udp <UDP_ADDRESS>  Bind server to given UDP address [default: 127.0.0.1:8081]
  -h, --help                      Print help
```
