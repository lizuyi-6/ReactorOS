/*
  ESP32 Reactor Bridge example for ReactorOS.

  Wire real sensors and actuators into the read* and applyTargets functions.
  This example keeps local safety responsibility on ESP32 even when commands
  come from Raspberry Pi.
*/

static uint32_t seqNo = 0;
static float targetTempC = 175.0f;
static float targetRpm = 450.0f;
static float targetShakeCpm = 30.0f;
static float targetPressureMpa = 0.5f;
static float heatTimeS = 300.0f;
static float holdTimeS = 600.0f;
static float coolTimeS = 180.0f;
static uint32_t lastCommandMs = 0;

uint8_t xorChecksum(const String &body) {
  uint8_t chk = 0;
  for (size_t i = 0; i < body.length(); i++) {
    chk ^= static_cast<uint8_t>(body[i]);
  }
  return chk;
}

String checksumHex(uint8_t value) {
  const char *digits = "0123456789ABCDEF";
  String out;
  out += digits[(value >> 4) & 0x0F];
  out += digits[value & 0x0F];
  return out;
}

void sendFrame(float tempC, float pressureMpa, float rpm, float shakeCpm, float flowLMin, float concentration, float ph) {
  String body = "RX|v=1";
  body += "|seq=" + String(seqNo++);
  body += "|ms=" + String(millis());
  body += "|temp=" + String(tempC, 2);
  body += "|pressure=" + String(pressureMpa, 2);
  body += "|stir_speed=" + String(rpm, 2);
  body += "|shake_speed=" + String(shakeCpm, 2);
  body += "|flow_rate=" + String(flowLMin, 2);
  body += "|product_concentration=" + String(concentration, 2);
  body += "|ph=" + String(ph, 2);
  Serial.println(body + "|chk=" + checksumHex(xorChecksum(body)));
}

bool parseTarget(const String &line, const String &key, float &out) {
  String needle = key + "=";
  int start = line.indexOf(needle);
  if (start < 0) return false;
  start += needle.length();
  int end = line.indexOf('|', start);
  if (end < 0) end = line.length();
  out = line.substring(start, end).toFloat();
  return true;
}

void handleCommand(const String &line) {
  if (!line.startsWith("TX|")) return;
  int chkStart = line.lastIndexOf("|chk=");
  if (chkStart < 0) return;
  String body = line.substring(0, chkStart);
  String expected = checksumHex(xorChecksum(body));
  String actual = line.substring(chkStart + 5);
  actual.trim();
  if (!actual.equalsIgnoreCase(expected)) return;

  float nextTemp = targetTempC;
  float nextRpm = targetRpm;
  float nextShake = targetShakeCpm;
  float nextPressure = targetPressureMpa;
  parseTarget(line, "heat_time", heatTimeS);
  parseTarget(line, "hold_time", holdTimeS);
  parseTarget(line, "cool_time", coolTimeS);
  if (parseTarget(line, "target_temp", nextTemp)) {
    targetTempC = constrain(nextTemp, 0.0f, 500.0f);
  }
  if (parseTarget(line, "stir_speed", nextRpm) || parseTarget(line, "target_rpm", nextRpm)) {
    targetRpm = constrain(nextRpm, 0.0f, 2000.0f);
  }
  if (parseTarget(line, "shake_speed", nextShake) || parseTarget(line, "target_shake", nextShake)) {
    targetShakeCpm = constrain(nextShake, 0.0f, 60.0f);
  }
  if (parseTarget(line, "target_pressure", nextPressure)) {
    targetPressureMpa = constrain(nextPressure, 0.0f, 10.0f);
  }
  lastCommandMs = millis();
  applyTargets(targetTempC, targetRpm, targetShakeCpm, targetPressureMpa);
}

float readTemperatureC() {
  return 175.0f + sin(millis() / 15000.0f) * 2.0f;
}

float readPressureMpa() {
  return 0.21f + sin(millis() / 19000.0f) * 0.01f;
}

float readRpm() {
  return targetRpm;
}

float readShakeCpm() {
  return targetShakeCpm;
}

float readFlowLMin() {
  return 2.5f + targetRpm / 1000.0f + targetShakeCpm / 100.0f;
}

float readConcentrationPercent() {
  return 62.4f + sin(millis() / 24000.0f) * 0.8f;
}

float readPh() {
  return 7.18f + sin(millis() / 31000.0f) * 0.05f;
}

void applyTargets(float tempC, float rpm, float shakeCpm, float pressureMpa) {
  // Replace with relay/PWM/VFD/control-loop output.
  // Keep hard safety interlocks here, independent of Raspberry Pi.
  (void)tempC;
  (void)rpm;
  (void)shakeCpm;
  (void)pressureMpa;
}

void enforceLocalSafety(float tempC) {
  if (tempC > 250.0f) {
    applyTargets(0.0f, 0.0f, 0.0f, 0.0f);
  }
  if (millis() - lastCommandMs > 30000UL) {
    // Communication watchdog: keep or reduce outputs according to lab policy.
  }
}

void setup() {
  Serial.begin(115200);
  lastCommandMs = millis();
}

void loop() {
  static uint32_t lastSampleMs = 0;
  while (Serial.available()) {
    String line = Serial.readStringUntil('\n');
    handleCommand(line);
  }

  float tempC = readTemperatureC();
  enforceLocalSafety(tempC);

  if (millis() - lastSampleMs >= 1000UL) {
    lastSampleMs = millis();
    sendFrame(
      tempC,
      readPressureMpa(),
      readRpm(),
      readShakeCpm(),
      readFlowLMin(),
      readConcentrationPercent(),
      readPh()
    );
  }
}
