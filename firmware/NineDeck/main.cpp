#include <Arduino.h>
// NineDeck test-jig firmware v0.9 — the screen shows the same pictures as Orvo Studio's 9 boxes.
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
//                   "A <dir>"             -> the next "F" slides the 3x3 in (1 = from the right, -1 = from the left)
//                   "V <ms>"              -> slide duration (0 = no slide)
//                   "N1 <text>"           -> name of what the slider controls (shown while moving it)
//                   "T 0" / "T 1"         -> slider is linear (B) / audio-taper (A)
//                   "C"                   -> calibrate the slider at 5 points (bottom, 1/4, middle, 3/4, top; key 1 confirms)
//                   "R 0" / "R 1"         -> slider direction normal / flipped
//                   "G <0-1000>" / "G -1" -> show / hide a "move here" mark on the level bar (pickup)
// Screen: the 3x3 boxes on the left; page keys top right; under the left page key a level bar for
// the slider (2 boxes tall); the space under the right page key is kept for a second slider.
//   device -> PC :  "S1 <0-1000>"         -> slider position (only when it moves)
// Slider: linear pot, ends on 3V3 and GND, wiper on GP26 (ADC0).
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
#include <tusb.h>
#include <EEPROM.h>

#define FW_VERSION "0.9.2"

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

// slide animation: the 3x3 area (GRID_W x SH) of the page being left
#define GRID_W (GX + 3 * BOX + 2 * GAP + 4)   // 176
uint16_t oldGrid[GRID_W * SH];
int slideDir = 0;          // set by "A", used by the next "F"
int slideMs = 180;         // set by "V"

// slider (GP26)
#define PIN_SLIDER 26
float sliderAvg = -1;      // smoothed reading, 0..4095
int sliderSent = -1;       // last value sent to the PC, 0..1000
uint32_t sliderMovedAt = 0;
bool sliderShown = false;
int sliderIcon = 0;       // what the slider controls: 0 nothing, 1 speakers, 2 mic, 3 an app
int pickupMark = -1;       // where to move the slider to take over (0..1000), -1 = none
char sliderName[24] = "Slider";

// Slider calibration, kept in flash. Without a calibration the pad learns the real ends
// as you slide; "audio" undoes the log curve of A-type pots.
struct SliderCal { uint32_t magic; float lo, mid, hi; uint8_t audio, hasMid; float pt[5]; uint8_t hasPts, flip; };
const uint32_t CAL_MAGIC = 0x4F525632;      // "ORV2"
SliderCal cal = {CAL_MAGIC, 4095, -1, 0, 0, 0, {0, 0, 0, 0, 0}, 0, 0};
bool calDirty = false;
uint32_t calChangedAt = 0;
int calStep = -1;                            // -1 = not calibrating, 0..4 = bottom .. top
void drawCalStep();

void loadCal() {
  EEPROM.begin(256);
  SliderCal c;
  EEPROM.get(0, c);
  if (c.magic == CAL_MAGIC && c.hi > c.lo) cal = c;
}
void saveCal() {
  EEPROM.put(0, cal);
  EEPROM.commit();
  calDirty = false;
}

// raw ADC (0..4095, smoothed) -> 0..1000
int sliderValue(float raw) {
  if (calStep < 0 && !cal.hasPts) {           // learn the real ends
    if (raw < cal.lo - 8 || raw > cal.hi + 8) {
      cal.lo = min(cal.lo, raw);
      cal.hi = max(cal.hi, raw);
      calDirty = true;
      calChangedAt = millis();
    }
  }
  float lo = cal.lo, hi = cal.hi;
  if (hi - lo < 600) { lo = 40; hi = 4055; }  // not learned yet
  float x;
  if (cal.hasPts) {                                          // 5-point calibration
    const float *p = cal.pt;
    if (raw <= p[0]) x = 0;
    else if (raw >= p[4]) x = 1;
    else {
      int i = 0;
      while (i < 3 && raw > p[i + 1]) i++;
      float span = p[i + 1] - p[i];
      x = (i + (span > 0 ? (raw - p[i]) / span : 0)) * 0.25f;
    }
  } else {
    float r = (raw - lo) / (hi - lo);
    if (r < 0) r = 0;
    if (r > 1) r = 1;
    // A-type pots: about 10 % of the signal at half travel; undo that curve
    x = cal.audio ? logf(1 + r * 80.0f) / logf(81.0f) : r;
  }
  int v = constrain((int)(x * 1000 + 0.5f), 0, 1000);
  return cal.flip ? 1000 - v : v;
}


uint16_t colBuf[SH];       // one screen column on its way to the display

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

// Small white icon (about 22x18) centred on cx, cy, at the bottom of the level bar.
void drawSpeaker(int16_t x, int16_t y, uint16_t c) {   // x, y = left middle
  frame.fillRect(x, y - 3, 4, 7, c);
  frame.fillTriangle(x + 3, y, x + 9, y - 7, x + 9, y + 7, c);
  frame.fillRect(x + 3, y - 3, 3, 7, c);
}
void drawSliderIcon(int16_t cx, int16_t cy) {
  uint16_t c = C_TEXT;
  if (sliderIcon == 1) {
    drawSpeaker(cx - 9, cy, c);
    frame.drawCircleHelper(cx + 1, cy, 5, 0x2 | 0x4, c);
    frame.drawCircleHelper(cx + 1, cy, 9, 0x2 | 0x4, c);
  } else if (sliderIcon == 2) {
    frame.fillRoundRect(cx - 3, cy - 9, 7, 11, 3, c);
    frame.drawCircleHelper(cx, cy - 2, 6, 0x4 | 0x8, c);
    frame.drawFastHLine(cx - 6, cy - 2, 2, c);
    frame.drawFastHLine(cx + 5, cy - 2, 2, c);
    frame.drawFastVLine(cx, cy + 4, 4, c);
    frame.drawFastHLine(cx - 4, cy + 8, 9, c);
  } else if (sliderIcon == 3) {
    frame.drawRoundRect(cx - 11, cy - 9, 22, 18, 3, c);
    frame.drawFastHLine(cx - 11, cy - 5, 22, c);
    drawSpeaker(cx - 6, cy + 2, c);
    frame.drawCircleHelper(cx + 3, cy + 2, 4, 0x2 | 0x4, c);
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
  // level bar under the left page key
  int16_t bx = PX, by = GY + PKH + GAP, bw = PKW, bh = SH - by - 2;
  int pct = sliderSent < 0 ? 0 : (sliderSent + 5) / 10;
  frame.fillRoundRect(bx, by, bw, bh, RADIUS, C_KEY);
  frame.drawRoundRect(bx, by, bw, bh, RADIUS, C_EDGE);
  int fh = (bh - 6) * pct / 100;
  if (fh > 0) frame.fillRoundRect(bx + 3, by + bh - 3 - fh, bw - 6, fh, RADIUS - 2, C_ACCENT);
  if (pickupMark >= 0) {                 // pickup: where the slider has to go to take over
    int my = by + bh - 3 - (bh - 6) * pickupMark / 1000;
    frame.fillRect(bx + 1, my - 1, bw - 2, 3, C_ORANGE);
  }
  char num[6];
  snprintf(num, sizeof num, "%d", pct);
  frame.setTextSize(2);
  frame.setTextColor(C_TEXT);
  int16_t x1, y1; uint16_t w, h;
  frame.getTextBounds(num, 0, 0, &x1, &y1, &w, &h);
  frame.setCursor(bx + (bw - (int)w) / 2, by + 8);
  frame.print(num);
  drawSliderIcon(bx + bw / 2, by + bh - 16);
  // (space under the right page key is kept for a second slider)

}

void snapshotGrid() {
  uint16_t *fb = frame.getBuffer();
  for (int y = 0; y < SH; y++) memcpy(&oldGrid[y * GRID_W], &fb[y * SW], GRID_W * 2);
}

float ease(float t) { return t < 0.5f ? 2 * t * t : 1 - 2 * (1 - t) * (1 - t); }

// Slides the new 3x3 (already composed in frame) over the old one (oldGrid).
void slideGrid(int dir) {
  uint16_t *fb = frame.getBuffer();
  uint32_t start = millis();
  for (;;) {
    float t = slideMs > 0 ? (millis() - start) / (float)slideMs : 1.0f;
    if (t > 1) t = 1;
    int o = (int)(ease(t) * GRID_W + 0.5f);       // how far the new page has come in
    tft.startWrite();
    for (int c = GRID_W - 1; c >= 0; c--) {
      // dir 1: new page enters from the right; dir -1: from the left
      int src = dir > 0 ? c + o : c - o;
      bool fromNew = dir > 0 ? src >= GRID_W : src < 0;
      if (fromNew) src = dir > 0 ? src - GRID_W : src + GRID_W;
      for (int r = 0; r < SH; r++) colBuf[r] = fromNew ? fb[r * SW + src] : oldGrid[r * GRID_W + src];
      tft.setAddrWindow(c, 0, 1, SH);
      tft.writePixels(colBuf, SH, true, false);
    }
    tft.endWrite();
    if (t >= 1) break;
  }
}

void composeAll() {
  frame.fillScreen(C_BG);
  for (int i = 0; i < 9; i++) composeBox(i);
  composePanel();
}

// The panel refreshes along the long side (it is a portrait panel turned sideways), so the
// picture is sent one column at a time in that same direction. Sent row by row (across the
// refresh) a change shows up as a diagonal tear; along it, at most a straight edge for one frame.
void pushColumns(int16_t x, int16_t y, int16_t w, int16_t h, const uint16_t *src, int stride, int srcX) {
  tft.startWrite();
  for (int c = w - 1; c >= 0; c--) {
    for (int r = 0; r < h; r++) colBuf[r] = src[(y + r) * stride + srcX + c];
    tft.setAddrWindow(x + c, y, 1, h);
    tft.writePixels(colBuf, h, true, false);
  }
  tft.endWrite();
}
void push(int16_t x, int16_t y, int16_t w, int16_t h) { pushColumns(x, y, w, h, frame.getBuffer(), SW, x); }
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

// ---------- Idle screen ----------
// When Orvo Studio isn't running (nothing heard from the PC for a few seconds; the app sends "h"
// every second), the pad shows the Orvo logo gently floating up and down instead of old pictures.
bool idle = true;
uint32_t lastHost = 0;

void ring(int16_t cx, int16_t cy, int16_t r, int16_t t, uint16_t c) {
  frame.fillCircle(cx, cy, r, c);
  frame.fillCircle(cx, cy, r - t, C_BG);
}

// The "Ports" O: a thick ring with four ports on the diagonals (a ring joined to the O, a gap,
// and a small donut inside).
void drawLogo(int16_t cx, int16_t cy) {
  const int16_t R = 34, T = 8, PR = 11;
  const float d = (R + PR - 3) * 0.7071f;
  for (int k = 0; k < 4; k++) {
    int16_t px = cx + (k & 1 ? d : -d), py = cy + (k & 2 ? d : -d);
    frame.fillCircle(px, py, PR, C_ACCENT);
  }
  ring(cx, cy, R, T, C_TEXT);
  for (int k = 0; k < 4; k++) {
    int16_t px = cx + (k & 1 ? d : -d), py = cy + (k & 2 ? d : -d);
    frame.fillCircle(px, py, PR - 3, C_BG);       // the gap
    ring(px, py, 5, 3, C_ACCENT);                 // the donut
  }
  frame.setTextSize(2);
  frame.setTextColor(C_TEXT);
  int16_t x1, y1; uint16_t w, h;
  frame.getTextBounds("ORVO", 0, 0, &x1, &y1, &w, &h);
  frame.setCursor(cx - w / 2, cy + R + 22);
  frame.print("ORVO");
}

const int16_t LOGO_X = 0, LOGO_W = SW;
void drawIdle(uint32_t now) {
  float phase = (now % 3000) / 3000.0f * 6.2832f;
  int16_t bob = (int16_t)(sinf(phase) * 6);        // +-6 px, one float every 3 s
  int16_t cx = SW / 2, cy = 66 + bob;
  frame.fillRect(LOGO_X, 0, LOGO_W, SH, C_BG);
  // soft shadow: smaller and fainter while the logo is higher
  int16_t sw = 30 + bob * 2;
  frame.fillRoundRect(cx - sw, SH - 12, sw * 2, 4, 2, 0x10A2);
  drawLogo(cx, cy);
  push(LOGO_X, 0, LOGO_W, SH);
}

void enterIdle() {
  idle = true;
  for (int i = 0; i < 9; i++) hasPicture[i] = false;   // don't bring back old pictures later
  pickupMark = -1;
}

void leaveIdle() {
  idle = false;
  composeAll();
  pushAll();
}

void setup() {
  // Backlight off first: at power-up the panel's memory holds random noise.
  pinMode(PIN_BL, OUTPUT);
  digitalWrite(PIN_BL, LOW);
  for (int i = 0; i < 11; i++) pinMode(KEY_PINS[i], INPUT_PULLUP);

  SPI.setSCK(PIN_SCK);
  SPI.setTX(PIN_MOSI);
  tft.init(172, 320);
  tft.setSPISpeed(62500000);
  tft.setRotation(1);               // landscape 320 x 172
  uint8_t rate = 0x01;              // FRCTRL2: ~111 Hz refresh instead of 60 Hz
  tft.sendCommand(0xC6, &rate, 1);

  analogReadResolution(12);
  loadCal();
  Serial.begin(115200);
  Serial.setTimeout(500);
  Keyboard.begin();

  drawIdle(millis());                // screen is clean now: light it up
  digitalWrite(PIN_BL, HIGH);
}

void handleSerial() {
  static char buf[80];
  static uint8_t n = 0;
  while (Serial.available()) {
    char c = Serial.read();
    if (c == '\r') continue;
    if (c != '\n' && n < sizeof buf - 1) { buf[n++] = c; continue; }
    buf[n] = 0; n = 0;
    lastHost = millis();
    if (buf[0] == 'h') {                              // heartbeat from Orvo Studio
      if (idle) leaveIdle();
      continue;
    }
    if (idle && buf[0] != '?') leaveIdle();

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
    } else if (buf[0] == 'A') {                       // next page change slides in
      slideDir = atoi(buf + 1) < 0 ? -1 : 1;
      snapshotGrid();
    } else if (buf[0] == 'T') {
      uint8_t a = atoi(buf + 1) ? 1 : 0;
      if (a != cal.audio) { cal.audio = a; saveCal(); sliderSent = -1; }
    } else if (buf[0] == 'G') {
      pickupMark = atoi(buf + 1);
      if (pickupMark > 1000) pickupMark = 1000;
      composePanel();
      push(PX, GY + PKH, SW - PX, SH - (GY + PKH));
    } else if (buf[0] == 'Y') {                       // icon of what the slider controls
      sliderIcon = atoi(buf + 1);
      composePanel();
      push(PX, GY + PKH, SW - PX, SH - (GY + PKH));
    } else if (buf[0] == 'R') {
      uint8_t f = atoi(buf + 1) ? 1 : 0;
      if (f != cal.flip) { cal.flip = f; saveCal(); sliderSent = -1; }
    } else if (buf[0] == 'C') {
      calStep = 0;
      drawCalStep();
    } else if (buf[0] == 'N' && buf[1] == '1') {
      strncpy(sliderName, buf[2] == ' ' ? buf + 3 : buf + 2, sizeof sliderName - 1);
      sliderName[sizeof sliderName - 1] = 0;
    } else if (buf[0] == 'V') {
      slideMs = constrain(atoi(buf + 1), 0, 1000);
    } else if (buf[0] == 'F') {
      composeAll();
      if (slideDir != 0 && slideMs > 0) {
        slideGrid(slideDir);
        push(GRID_W, 0, SW - GRID_W, SH);
      } else {
        pushAll();
      }
      slideDir = 0;
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

// Screen off while the PC is asleep or the USB cable carries power only.
void watchHost(uint32_t now) {
  static uint32_t last = 0;
  static bool lit = true;
  if (now - last < 250) return;
  last = now;
  bool awake = tud_mounted() && !tud_suspended();
  if (awake != lit) {
    lit = awake;
    digitalWrite(PIN_BL, awake ? HIGH : LOW);
  }
}

// on-screen calibration steps
void drawCalStep() {
  static const char *steps[] = {"Slide to the BOTTOM", "Slide to 1/4", "Slide to the MIDDLE", "Slide to 3/4", "Slide to the TOP"};
  frame.fillRect(PX, GY + PKH + 4, SW - PX, SH - (GY + PKH + 4), C_BG);
  frame.setTextColor(C_ORANGE);
  frame.setTextSize(1);
  frame.setCursor(PX + 6, GY + PKH + 16);
  frame.print("SLIDER CALIBRATION");
  frame.setTextColor(C_TEXT);
  frame.setCursor(PX + 6, GY + PKH + 36);
  frame.print(steps[calStep]);
  frame.setTextColor(C_DIM);
  frame.setCursor(PX + 6, GY + PKH + 56);
  frame.print("then press key 1");
  char st[8];
  snprintf(st, sizeof st, "%d / 5", calStep + 1);
  frame.setCursor(PX + 6, SH - 16);
  frame.print(st);
  push(PX, 0, SW - PX, SH);
}

// Reads the slider every 2 ms and smooths it lightly. A new value is only taken when the slider
// really moved (hysteresis), and the last ~1 % at each end snaps to exactly 0 / 100, so the number
// never flickers between neighbours. The level bar on the screen is redrawn on every real change.
void readSlider(uint32_t now) {
  static uint32_t last = 0, drawnAt = 0;
  static float held = -1;                 // the position currently reported, 0..1000 (float, pre-rounding)
  static bool dirty = false;
  if (now - last < 2) return;
  last = now;
  int raw = analogRead(PIN_SLIDER);
  sliderAvg = sliderAvg < 0 ? raw : sliderAvg * 0.7f + raw * 0.3f;
  if (calStep >= 0) return;               // calibrating: don't report
  float v = sliderValue(sliderAvg);
  if (v <= 10) v = 0;                     // snap the ends
  if (v >= 990) v = 1000;
  if (calDirty && now - calChangedAt > 3000) saveCal();   // learned ends: save once you stop
  // hysteresis: move only when the new reading is 0.6 % away from the held one (or reaches an end)
  if (held < 0 || fabsf(v - held) >= 6 || ((v == 0 || v == 1000) && v != held)) {
    held = v;
    int out = (int)(v + 0.5f);
    if (out != sliderSent) {
      sliderSent = out;
      Serial.print("S1 "); Serial.println(out);
      dirty = true;
    }
  }
  if (dirty && !idle && now - drawnAt > 16) {      // redraw the level bar (at most ~60 times a second)
    drawnAt = now;
    dirty = false;
    composePanel();
    push(PX, GY + PKH, SW - PX, SH - (GY + PKH));
  }
}

void loop() {
  uint32_t now = millis();
  watchHost(now);
  if (!idle && lastHost && now - lastHost > 3500) enterIdle();
  static uint32_t idleDrawn = 0;
  if (idle && now - idleDrawn >= 33) {           // ~30 frames a second
    idleDrawn = now;
    drawIdle(now);
  }
  readSlider(now);
  for (int i = 0; i < 11; i++) {
    bool p = digitalRead(KEY_PINS[i]) == LOW;
    if (p != state[i] && now - changedAt[i] > 8) {     // 8 ms debounce
      state[i] = p;
      changedAt[i] = now;
      if (calStep >= 0) {                     // calibrating: key 1 confirms the step
        if (i == 0 && p) {
          cal.pt[calStep] = sliderAvg;
          if (++calStep > 4) {
            calStep = -1;
            // points measured with the pot mounted the other way round: store them low to high
            if (cal.pt[0] > cal.pt[4]) for (int a = 0, b = 4; a < b; a++, b--) { float t = cal.pt[a]; cal.pt[a] = cal.pt[b]; cal.pt[b] = t; }
            bool ok = true;
            for (int k = 0; k < 4; k++) if (cal.pt[k + 1] <= cal.pt[k]) ok = false;
            cal.hasPts = ok;
            cal.lo = cal.pt[0];
            cal.hi = cal.pt[4];
            saveCal();
            sliderSent = -1;
            composePanel();
            push(PX, 0, SW - PX, SH);
          } else {
            drawCalStep();
          }
        }
        continue;
      }
      Serial.print('K'); Serial.print(i + 1); Serial.println(p ? " 1" : " 0");
      if (hidOn) { if (p) Keyboard.press(KEY_CODES[i]); else Keyboard.release(KEY_CODES[i]); }
      // instant press feedback, drawn on the pad itself
      if (idle) {}
      else if (i < 9) { composeBox(i); push(boxX(i), boxY(i), BOX, BOX); }
      else { composePanel(); push(PX, 0, SW - PX, SH); }
    }
  }
  handleSerial();
}
