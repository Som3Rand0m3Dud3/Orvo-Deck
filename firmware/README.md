# NineDeck firmware (test jig, Raspberry Pi Pico)

Built with the arduino-pico core (Earle Philhower), board "Raspberry Pi Pico", with
Adafruit GFX + Adafruit ST7789. The built `.uf2` ships inside the NineDeck plugin
(`plugins/com.ninedeck.device.sdPlugin/assets/firmware/ninedeck-fw.uf2`) and is installed
from Orvo Studio with the **Update pad** button. When changing the firmware, bump the
version it reports (`NINEDECK x.y`) and `latestFirmware` in the plugin's `main.go`.
