[![Continuous Integration](https://github.com/helium/virtual-lorawan-device/actions/workflows/rust.yml/badge.svg)](https://github.com/helium/virtual-lorawan-device/actions/workflows/rust.yml)

# virtual-lorawan-device

If you need the legacy project, please see [here](https://github.com/helium/virtual-lorawan-device/tree/legacy).

## Configuration

You'll want to create a file called `settings.toml` and define one more devices.

For example:
```toml
# optionally override host
host = "127.0.0.1:1691"

[devices.one]
# each device is a GWMP Client with MAC 1:1
mac = "0807060504030201"
[devices.one.credentials]
mac_id = "3ED43BEF1857EE4B"
dev_eui = "3ED43BEF1857EE4B"
app_eui = "35BEED137ACD344B"
app_key = "275AD3615ACB47A381E6B79A832CC5AE"
```