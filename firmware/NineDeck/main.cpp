#include <Arduino.h>
// NineDeck test-jig firmware v0.4 — the screen shows the same pictures as Orvo Studio's 9 boxes.
// Board: Raspberry Pi Pico (RP2040), arduino-pico core (Earle Philhower)
// Keys: GP0-GP8 = Key1-9, GP9 = Page Left, GP10 = Page Right (each key to GND, internal pull-ups)
// Display: 1.47" 172x320 ST7789 on SPI0: SCL GP18, SDA GP19, CS GP17, DC GP16, RES GP20, BLK GP21
//
// USB: a keyboard (Key1-9 -> F13-F21, Page L -> F22, Page R -> F23, until the app turns it off)
//      and a serial port (115200) for the Orvo NineDeck plugin:
//   device -> PC :  "K<n> 1" / "K<n> 0"   key n (1..11) pressed / released
//   PC -> device :  "?"                   -> "NINEDECK 0.3 KEYS 11 BOX <size>"
//                   "I<n> <w> <h>" + w*h*2 bytes (RGB565, little endian) -> picture for box n (1..9)
//                   "Z<n> <w> <h> <len>" + len bytes -> same picture, packed as runs of
//                                            [count][low][high] (much smaller for icons)
//                   "E<n>"                -> box n is empty
//                   "P <text>"            -> page name shown next to the page keys
//                   "F"                   -> show everything sent so far, all at once
//                   "L<n> <text>"         -> (old) text label, drawn as a picture
//                   "B"                   -> restart into the USB update drive (RPI-RP2)
//                   "H 0" / "H 1"         -> F13-F23 keyboard presses off / on
//
// The whole screen is built in memory first and sent to the display in one burst, so a
// page change appears at once instead of box by box.

#include <SPI.h>
#include <Adafruit_GFX.h>
#include <Adafruit_ST7789.h>
#include <Keyboard.h>

#define FW_VERSION "0.4"

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

// screen (landscape) and layout
#define SW 320
#define SH 172
#define BOX 52          // box size in pixels (square, like the app's boxes)
#define GAP 6
#define GX 4            // grid origin
#define GY 2
#define RADIUS 7

// colours (RGB565)
#define C_BG      0x0841
#define C_KEY     0x18E3
#define C_EDGE    0x3186
#define C_TEXT    0xFFFF
#define C_DIM     0x8C71
#define C_ACCENT  0x4C7F   // Orvo blue
#define C_ORANGE  0xFC47

GFXcanvas16 frame(SW, SH);
uint16_t boxes[9][BOX * BOX];
bool hasPicture[9];
char pageName[40] = "";

bool state[11];
uint32_t changedAt[11];
bool hidOn = true;

int16_t boxX(int i) { return GX + (i % 3) * (BOX + GAP); }
int16_t boxY(int i) { return GY + (i / 3) * (BOX + GAP); }

// right-hand panel: page keys and page name
const int16_t PX = GX + 3 * BOX + 2 * GAP + 8;     // 180
const int16_t PW = SW - PX - 4;                     // 136
const int16_t PKW = (PW - GAP) / 2;                 // page key width
const int16_t PKH = BOX;

bool inCorner(int x, int y) {        // rounded-corner mask
  int r = RADIUS;
  int cx = x < r ? r - 1 : (x >= BOX - r ? BOX - r : -1);
  int cy = y < r ? r - 1 : (y >= BOX - r ? BOX - r : -1);
  if (cx < 0 || cy < 0) return false;
  int dx = x - cx, dy = y - cy;
  return dx * dx + dy * dy > r * r;
}

void composeBox(int i) {
  int16_t x0 = boxX(i), y0 = boxY(i);
  uint16_t *fb = frame.getBuffer();
  for (int y = 0; y < BOX; y++) {
    for (int x = 0; x < BOX; x++) {
      uint16_t c;
      if (inCorner(x, y)) c = C_BG;
      else if (hasPicture[i]) c = boxes[i][y * BOX + x];
      else c = C_KEY;
      fb[(y0 + y) * SW + x0 + x] = c;
    }
  }
  if (!hasPicture[i]) frame.drawRoundRect(x0, y0, BOX, BOX, RADIUS, C_EDGE);
  if (state[i]) {
    frame.drawRoundRect(x0, y0, BOX, BOX, RADIUS, C_ACCENT);
    frame.drawRoundRect(x0 + 1, y0 + 1, BOX - 2, BOX - 2, RADIUS - 1, C_ACCENT);
  }
}

void drawArrow(int16_t x, int16_t y, bool left, uint16_t c) {
  int16_t cx = x + PKW / 2, cy = y + PKH / 2;
  for (int t = -1; t <= 1; t++) {
    if (left) { frame.drawLine(cx + 5 + t, cy - 9, cx - 4 + t, cy, c); frame.drawLine(cx - 4 + t, cy, cx + 5 + t, cy + 9, c); }
    else      { frame.drawLine(cx - 5 + t, cy - 9, cx + 4 + t, cy, c); frame.drawLine(cx + 4 + t, cy, cx - 5 + t, cy + 9, c); }
  }
}

void composePanel() {
  frame.fillRect(PX, 0, SW - PX, SH, C_BG);
  for (int k = 0; k < 2; k++) {
    int16_t x = PX + k * (PKW + GAP), y = GY;
    bool p = state[9 + k];
    frame.fillRoundRect(x, y, PKW, PKH, RADIUS, p ? C_ORANGE : C_KEY);
    frame.drawRoundRect(x, y, PKW, PKH, RADIUS, p ? C_TEXT : C_EDGE);
    drawArrow(x, y, k == 0, p ? C_BG : C_TEXT);
  }
  // page name, wrapped onto up to 3 lines
  frame.setTextColor(C_TEXT);
  frame.setTextSize(2);
  frame.setTextWrap(false);
  int16_t y = GY + PKH + 18;
  const char *s = pageName;
  char line[12];
  for (int row = 0; row < 3 && *s; row++) {
    int n = 0;
    while (s[n] && n < 11) n++;          // 11 characters of size-2 text fit in the panel
    if (s[n] && n == 11) {               // break at a space if we can
      int b = n;
      while (b > 0 && s[b] != ' ') b--;
      if (b > 0) n = b;
    }
    memcpy(line, s, n); line[n] = 0;
    int16_t x1, y1; uint16_t w, h;
    frame.getTextBounds(line, 0, 0, &x1, &y1, &w, &h);
    frame.setCursor(PX + (PW - (int)w) / 2, y);
    frame.print(line);
    y += 20;
    s += n;
    while (*s == ' ') s++;
  }
  frame.setTextSize(1);
  frame.setTextColor(C_DIM);
  frame.setCursor(PX + (PW - 6 * 4) / 2, SH - 12);
  frame.print("ORVO");
}

void composeAll() {
  frame.fillScreen(C_BG);
  for (int i = 0; i < 9; i++) composeBox(i);
  composePanel();
}

void push(int16_t x, int16_t y, int16_t w, int16_t h) {
  uint16_t *fb = frame.getBuffer();
  tft.startWrite();
  tft.setAddrWindow(x, y, w, h);
  for (int r = 0; r < h; r++) tft.writePixels(&fb[(y + r) * SW + x], w, true, false);
  tft.endWrite();
}
void pushAll() { push(0, 0, SW, SH); }

// Old text labels ("L<n> text") are turned into a simple picture so older plugins still work.
void labelToPicture(int i, const char *text) {
  GFXcanvas16 c(BOX, BOX);
  c.fillScreen(C_KEY);
  c.setTextColor(C_TEXT);
  c.setTextSize(strlen(text) > 7 ? 1 : 2);
  c.setTextWrap(false);
  int16_t x1, y1; uint16_t w, h;
  c.getTextBounds(text, 0, 0, &x1, &y1, &w, &h);
  c.setCursor((BOX - (int)w) / 2, (BOX - (int)h) / 2);
  c.print(text);
  memcpy(boxes[i], c.getBuffer(), sizeof boxes[i]);
  hasPicture[i] = text[0] != 0;
}

void splash() {
  frame.fillScreen(C_BG);
  frame.setTextColor(C_TEXT);
  frame.setTextSize(4);
  int16_t x1, y1; uint16_t w, h;
  frame.getTextBounds("NineDeck", 0, 0, &x1, &y1, &w, &h);
  frame.setCursor((SW - w) / 2, 52);
  frame.print("NineDeck");
  frame.fillRoundRect((SW - 120) / 2, 104, 120, 4, 2, C_ACCENT);
  frame.setTextSize(1);
  frame.setTextColor(C_DIM);
  frame.setCursor((SW - 6 * 18) / 2, 124);
  frame.print("test jig  fw v" FW_VERSION);
  pushAll();
}

void setup() {
  for (int i = 0; i < 11; i++) pinMode(KEY_PINS[i], INPUT_PULLUP);

  pinMode(PIN_BL, OUTPUT);
  digitalWrite(PIN_BL, HIGH);

  SPI.setSCK(PIN_SCK);
  SPI.setTX(PIN_MOSI);
  tft.init(172, 320);
  tft.setSPISpeed(62500000);
  tft.setRotation(1);               // landscape 320 x 172

  Serial.begin(115200);
  Serial.setTimeout(500);
  Keyboard.begin();

  splash();
  delay(1000);
  for (int i = 0; i < 9; i++) {
    char t[3];
    snprintf(t, sizeof t, "%d", i + 1);
    labelToPicture(i, t);
  }
  composeAll();
  pushAll();
}

void handleSerial() {
  static char buf[80];
  static uint8_t n = 0;
  while (Serial.available()) {
    char c = Serial.read();
    if (c == '\r') continue;
    if (c != '\n' && n < sizeof buf - 1) { buf[n++] = c; continue; }
    buf[n] = 0; n = 0;

    if (buf[0] == '?') {
      Serial.print("NINEDECK " FW_VERSION " KEYS 11 BOX ");
      Serial.println(BOX);
    } else if (buf[0] == 'I') {                       // I<n> <w> <h> + pixels
      int k = 0, w = 0, h = 0;
      sscanf(buf + 1, "%d %d %d", &k, &w, &h);
      size_t len = (size_t)w * h * 2;
      if (k >= 1 && k <= 9 && w == BOX && h == BOX) {
        size_t got = Serial.readBytes((char *)boxes[k - 1], len);
        hasPicture[k - 1] = got == len;
      } else {                                        // wrong size: read and drop it
        for (size_t i = 0; i < len; i++) { uint32_t t0 = millis(); while (!Serial.available() && millis() - t0 < 500) {} if (Serial.available()) Serial.read(); }
      }
    } else if (buf[0] == 'Z') {                       // Z<n> <w> <h> <len> + packed pixels
      static uint8_t packed[BOX * BOX * 3];
      int k = 0, w = 0, h = 0, len = 0;
      sscanf(buf + 1, "%d %d %d %d", &k, &w, &h, &len);
      if (len < 0 || len > (int)sizeof packed) len = 0;
      size_t got = Serial.readBytes((char *)packed, len);
      if (k >= 1 && k <= 9 && w == BOX && h == BOX && got == (size_t)len) {
        uint16_t *dst = boxes[k - 1];
        int out = 0;
        for (int i = 0; i + 2 < len && out < BOX * BOX; i += 3) {
          uint16_t c = packed[i + 1] | (packed[i + 2] << 8);
          for (int r = 0; r < packed[i] && out < BOX * BOX; r++) dst[out++] = c;
        }
        hasPicture[k - 1] = out == BOX * BOX;
      }
    } else if (buf[0] == 'E') {
      int k = atoi(buf + 1);
      if (k >= 1 && k <= 9) hasPicture[k - 1] = false;
    } else if (buf[0] == 'P') {
      strncpy(pageName, buf[1] == ' ' ? buf + 2 : buf + 1, sizeof pageName - 1);
      pageName[sizeof pageName - 1] = 0;
    } else if (buf[0] == 'F') {
      composeAll();
      pushAll();
    } else if (buf[0] == 'L') {                       // old text labels
      int k = atoi(buf + 1);
      char *sp = strchr(buf, ' ');
      if (k >= 1 && k <= 9) {
        labelToPicture(k - 1, sp ? sp + 1 : "");
        composeBox(k - 1);
        push(boxX(k - 1), boxY(k - 1), BOX, BOX);
      }
    } else if (buf[0] == 'B') {                       // update requested by Orvo Studio
      Keyboard.releaseAll();
      frame.fillScreen(C_BG);
      frame.setTextColor(C_TEXT);
      frame.setTextSize(2);
      frame.setCursor(94, 76);
      frame.print("Updating...");
      pushAll();
      Serial.println("OK");
      Serial.flush();
      delay(100);
      rp2040.rebootToBootloader();
    } else if (buf[0] == 'H') {
      hidOn = (buf[2] == '1');
      if (!hidOn) Keyboard.releaseAll();
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
      // instant press feedback, drawn on the pad itself
      if (i < 9) { composeBox(i); push(boxX(i), boxY(i), BOX, BOX); }
      else { composePanel(); push(PX, 0, SW - PX, SH); }
    }
  }
  handleSerial();
}
