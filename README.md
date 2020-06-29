[![](https://travis-ci.com/helium/virtual-lorawan-device.svg?token=35YrBmyVB8LNrXzjrRop&branch=master)](https://travis-ci.com/helium/virtual-lorawan-device)
# virtual-lorawan-device
Download a compiled release [here](https://github.com/helium/virtual-lorawan-device/releases).
# Features
This utility replaces a Semtech forwarder with this utility which instantiates
one or more virtual LoRaWAN devices. You can load devices in two ways: using
a local file (default) or by pulling devices from Console.

To use a local file, create a file called `lorawan-devices.json` (you can
override the name with CLI option) in the following format:
```json
{
    "devices":
    [
      {
        "credentials": {
          "app_eui": "70B3D57ED00294B9",
          "app_key": "BF40D30E4E23428EF682CA7764CDB423",
          "dev_eui": "003CC5371EB66C55"
        },
        "oui": 1,
        "transmit_delay": 10000
      }
    ]
}
```
You'll want to make sure the credentials match some devices on Console.

If you want to pull credentials from Console (Staging or Prod), you need
to create a file called `console-credentials.json` which should look like this:

```json
{
  "staging": "API_KEY",
  "production": "API_KEY"
}
```

You may ignore the field of the console you are not using (ie: just include "staging"
if you are running against staging). We also limit devices to 32 by default, but
the global `--max-devices` option let's you override that.

Note, that if you want to create lots of devices on Console, you can use 
[the CLI](https://github.com/helium/helium-console-cli). The command `device 
create-by-app-eui` is particularly useful for generating N devices when you don't 
care about name or credentials. 
Also note that the only way to point the CLI to staging is by editing the 
`.helium-console-config.toml` by hand.

Finally, you should know that when you run the utility by using devices from console,
the devices get printed to the terminal output in a format that is friendly for
`lorawan-devices.json`. Therefore, copy from there and making curated lists devices
after running them from an automatic import session can be done easily.

# Miner Setup
This utility is directly compatible with [a Miner that can be easily deployed using Docker](https://developer.helium.com/blockchain/run-your-own-miner). Note that state channel test mode requires that the Miner be added to the blockchain.
