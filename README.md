[![Continuous Integration](https://github.com/helium/virtual-lorawan-device/actions/workflows/rust.yml/badge.svg)](https://github.com/helium/virtual-lorawan-device/actions/workflows/rust.yml)

# virtual-lorawan-device

This utility allows you to run one or more LoRaWAN devices in pure software (ie: virtually).
It leverages [a Rust-based LoRaWAN device stack](https://github.com/ivajloip/rust-lorawan)
and implements its "Radio trait" with the [Semtech GWMP over UDP](https://github.com/helium/semtech-udp)
interface (see `udp_radio.rs`). This allows you to attach to any Semtech GWMP over UDP Host, 
such as a traditional LoRaWAN Network Server (LNS), [Helium Miner](https://github.com/helium/miner),
or the [Helium Light Hotspot](https://github.com/helium/gateway-rs).

To the Semtech GWMP over UDP Host, the default configuration of this utility looks like a 
single packet forwarder with one or more devices. It is possible to configure many packet
forwarders with many different hosts as well. PLease see the configuration examples below.

## Configuration

You'll want to create a file called `settings.toml` and define one more devices. By default,
this file is expected in the `settings` directory from where the application is launched. This
may be overriden with the `--config` option.

### A simple configuration

If you want to run one or more virtual devices, your `settings.toml` file may look like this:
```toml
# Optionally override host IP
#[packet_forwarder.default]
#host = "127.0.0.1:1681"

[devices.one.credentials]
dev_eui = "3ED43BEF1857EF4B"
app_eui = "35BEED137AC3344B"
app_key = "275AD3615ACA47A381E6B79A832CC5AE"

[devices.two.credentials]
dev_eui = "3ED43BEF18D7EE4B"
app_eui = "35BEED137ACD384B"
app_key = "275AD3615ACB47AA81E6B79A832CC5AE"
```

A single "virtual packet forwarder" will be instantiated and it will connect to the `default_host`.
The two devices will transmit and receive their packets via the single packet forwarder.

### A more complicated configuration

More complicated configurations are possible. You could have multiple virtual packet forwarders:

```toml
[packet_forwarder.pf_one]
mac = "0807060504030201"
host = "127.0.0.1:1691"

[packet_forwarder.pf_two]
mac = "0807060504030202"
host = "127.0.0.1:1692"

[device.one]
packet_forwarder = "pf_one"
oui = "1"
[devices.one.credentials]
dev_eui = "3ED43BEF1857EF4B"
app_eui = "35BEED137AC3344B"
app_key = "275AD3615ACA47A381E6B79A832CC5AE"

[device.two]
packet_forwarder = "pf_two"
oui = "2"
[devices.two.credentials]
dev_eui = "3ED43BEF18D7EE4B"
app_eui = "35BEED137ACD384B"
app_key = "275AD3615ACB47AA81E6B79A832CC5AE"
```

In this configuration, we've created two packet forwarders and attached one device to each. In addition,
we've given them different `oui` labels. This will put their data reported to Prometheus under different
labels.
