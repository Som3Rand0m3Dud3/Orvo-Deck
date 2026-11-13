#include <Arduino.h>
// NineDeck test-jig firmware v0.2 (faster, tear-free key drawing; update from Orvo Studio)
// Board: Raspberry Pi Pico (RP2040), arduino-pico core (Earle Philhower)
// Keys: GP0-GP8 = Key1-9, GP9 = Page Left, GP10 = Page Right (each key to GND, internal pull-ups)
// Display: 1.47" 172x320 ST7789 on SPI0: SCL GP18, SDA GP19, CS GP17, DC GP16, RES GP20, BLK GP21
//
// USB: acts as a keyboard (Key1-9 -> F13-F21, Page L -> F22, Page R -> F23)
//      and a serial port (115200) for the companion app:
//        device -> PC :  "K<n> 1" key pressed, "K<n> 0" key released   (n = 1..11)
//        PC -> device :  "?"                -> "NINEDECK 0.2 KEYS 11"
//                        "B"                -> restart into the USB update drive (RPI-RP2)
//                        "L<n> <text>"      -> set the label shown on key n
//                        "H 0" / "H 1"      -> turn the F13-F23 keyboard presses off / on

#include <SPI.h>
#include <Adafruit_GFX.h>
#include <Adafruit_ST7789.h>
#include <Keyboard.h>

#define PIN_DC   16
#define PIN_CS   17
#define PIN_SCK  18
#define PIN_MOSI 19
#define PIN_RST  20
#define PIN_BL   21

const uint8_t KEY_PINS[11] = {2, 1, 0, 5, 4, 3, 8, 7, 6, 9, 10};   // columns mirrored to match the jig
const uint8_t KEY_CODES[11] = {KEY_F13, KEY_F14, KEY_F15, KEY_F16, KEY_F17, KEY_F18,
                               KEY_F19, KEY_F20, KEY_F21, KEY_F22, KEY_F23};

Adafruit_ST7789 tft(&SPI, PIN_CS, PIN_DC, PIN_RST);

// colours (RGB565)
#define C_BG      0x0841
#define C_KEY     0x2124
#define C_EDGE    0x4A69
#define C_TEXT    0xFFFF
#define C_DIM     0x9CF3
#define C_ACCENT  0x2D7F   // blue
#define C_PAGE    0xFD20   // orange

char labels[11][12];
bool state[11];
uint32_t changedAt[11];
bool hidOn = true;

// layout in landscape (320 x 172)
struct Box { int16_t x, y, w, h; };
Box boxFor(int i) {
  if (i < 9) {                       // 3x3 grid on the left
    int c = i % 3, r = i / 3;
    return {(int16_t)(8 + c * 72), (int16_t)(8 + r * 54), 66, 48};
  }
  return {(int16_t)(232), (int16_t)(i == 9 ? 8 : 90), 80, 74};   // page keys on the right
}

// Each box is drawn off-screen first, then sent to the display in one burst,
// so it changes all at once instead of wiping across.
GFXcanvas16 gridCanvas(66, 48);
GFXcanvas16 pageCanvas(80, 74);

void drawKey(int i) {
  Box b = boxFor(i);
  GFXcanvas16 &cv = i < 9 ? gridCanvas : pageCanvas;
  bool p = state[i];
  uint16_t fill = p ? (i < 9 ? C_ACCENT : C_PAGE) : C_KEY;
  cv.fillScreen(C_BG);
  cv.fillRoundRect(0, 0, b.w, b.h, 8, fill);
  cv.drawRoundRect(0, 0, b.w, b.h, 8, p ? C_TEXT : C_EDGE);
  cv.setTextColor(p ? C_TEXT : C_DIM);
  cv.setTextSize(1);
  cv.setCursor(6, 5);
  if (i < 9) { cv.print('K'); cv.print(i + 1); }
  else cv.print("PAGE");
  cv.setTextColor(C_TEXT);
  cv.setTextSize(strlen(labels[i]) > 5 ? 1 : 2);
  int16_t x1, y1; uint16_t w, h;
  cv.getTextBounds(labels[i], 0, 0, &x1, &y1, &w, &h);
  cv.setCursor((b.w - (int)w) / 2, (b.h - (int)h) / 2 + 5);
  cv.print(labels[i]);
  tft.drawRGBBitmap(b.x, b.y, cv.getBuffer(), b.w, b.h);
}

void drawAll() {
  tft.fillScreen(C_BG);
  for (int i = 0; i < 11; i++) drawKey(i);
}

void splash() {
  tft.fillScreen(C_BG);
  tft.setTextColor(C_TEXT);
  tft.setTextSize(4);
  int16_t x1, y1; uint16_t w, h;
  tft.getTextBounds("NineDeck", 0, 0, &x1, &y1, &w, &h);
  tft.setCursor((320 - w) / 2, 52);
  tft.print("NineDeck");
  tft.fillRoundRect((320 - 120) / 2, 104, 120, 4, 2, C_ACCENT);
  tft.setTextSize(1);
  tft.setTextColor(C_DIM);
  tft.setCursor((320 - 6 * 19) / 2, 124);
  tft.print("test jig  fw v0.2");
}

void setup() {
  for (int i = 0; i < 11; i++) {
    pinMode(KEY_PINS[i], INPUT_PULLUP);
    snprintf(labels[i], sizeof labels[i], "%d", i + 1);
  }
  strcpy(labels[9], "<");
  strcpy(labels[10], ">");

  pinMode(PIN_BL, OUTPUT);
  digitalWrite(PIN_BL, HIGH);

  SPI.setSCK(PIN_SCK);
  SPI.setTX(PIN_MOSI);
  tft.init(172, 320);
  tft.setSPISpeed(62500000);
  tft.setRotation(1);               // landscape 320 x 172

  Serial.begin(115200);
  Keyboard.begin();

  splash();
  delay(1200);
  drawAll();
}

void handleSerial() {
  static char buf[64];
  static uint8_t n = 0;
  while (Serial.available()) {
    char c = Serial.read();
    if (c == '\r') continue;
    if (c != '\n' && n < sizeof buf - 1) { buf[n++] = c; continue; }
    buf[n] = 0; n = 0;
    if (buf[0] == '?') {
      Serial.println("NINEDECK 0.2 KEYS 11");
    } else if (buf[0] == 'L') {            // L<n> <text>
      int k = atoi(buf + 1);
      char *sp = strchr(buf, ' ');
      if (k >= 1 && k <= 11) {
        strncpy(labels[k - 1], sp ? sp + 1 : "", sizeof labels[0] - 1);
        labels[k - 1][sizeof labels[0] - 1] = 0;
        drawKey(k - 1);
        Serial.println("OK");
      }
    } else if (buf[0] == 'B') {           // update requested by Orvo Studio
      Keyboard.releaseAll();
      tft.fillScreen(C_BG);
      tft.setTextColor(C_TEXT);
      tft.setTextSize(2);
      tft.setCursor(70, 76);
      tft.print("Updating...");
      Serial.println("OK");
      Serial.flush();
      delay(100);
      rp2040.rebootToBootloader();
    } else if (buf[0] == 'H') {
      hidOn = (buf[2] == '1');
      if (!hidOn) Keyboard.releaseAll();
      Serial.println("OK");
    }
  }
}

void loop() {
  uint32_t now = millis();
  for (int i = 0; i < 11; i++) {
    bool p = digitalRead(KEY_PINS[i]) == LOW;
    if (p != state[i] && now - changedAt[i] > 8) {     // 8 ms debounce
      state[i] = p;
      changedAt[i] = now;
      Serial.print('K'); Serial.print(i + 1); Serial.println(p ? " 1" : " 0");
      if (hidOn) { if (p) Keyboard.press(KEY_CODES[i]); else Keyboard.release(KEY_CODES[i]); }
      drawKey(i);
    }
  }
  handleSerial();
}
