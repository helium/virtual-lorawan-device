[![](https://travis-ci.com/helium/virtual-lorawan-device.svg?token=35YrBmyVB8LNrXzjrRop&branch=master)](https://travis-ci.com/helium/virtual-lorawan-device)
# virtual-lorawan-device
Download a compiled release [here](https://github.com/helium/virtual-lorawan-devicec/releases).
# Features
This utility replaces a Semtech forwarder with this virtual LoRaWAN device instance.
Create a file called `lorawan-devices.json`, in the following format:
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
    ],
    "gateways":
    [
      "112CgbghEZwMwbKUXfz9i9o4Ysxtio4ucGH24zFNYRRU6V2RtJyk"
    ] 
}
```
You'll want to make sure the credentials match some devices on Console.
The gateways field is optional, as it is only relevant for the "state channel test" mode.
In "state channel test", the utility will detect the open state channel by using the blockchain-api.
If there are more than 3 blocks left, it will Join and starting sending packets.
When there are less than 3 blocks left, the device will stop sending packets and the utility will block until the state channel close transaction gets posted.
The output compares the amount of packets it sent versus what was posted in the channel on behalf of the gateway from the JSON configuration file.
# Miner Setup
This utility is directly compatible with [a Miner that can be easily deployed using Docker](https://developer.helium.com/blockchain/run-your-own-miner). Note that state channel test mode requires that the Miner be added to the blockchain.