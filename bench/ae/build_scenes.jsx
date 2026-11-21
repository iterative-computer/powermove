// After Effects equivalent of bench/scenes.js (Scenes A, B, C). UNTESTED: AE was not run (owner decision). Run with:
//   osascript -e 'tell application "Adobe After Effects 2027" to DoScriptFile "<this file>"'
// Reads $.getenv-less config from bench_config.txt beside this file: line 1 = media dir, line 2 = output dir.
// Saves scene-A.aep, scene-B.aep, scene-C.aep and queues nothing (aerender renders each comp by name).
(function () {
  var here = File($.fileName).parent;
  var cfg = File(here.fsName + "/bench_config.txt"); cfg.open("r");
  var MEDIA = cfg.readln(), OUT = cfg.readln(); cfg.close();
  var W = 1920, H = 1080, FPS = 30, DUR = 10;
  var PAL = ["#FF6B1A", "#4C8DFF", "#3FCF8E", "#F2C14E", "#E8E2CF", "#C04CFF", "#FF4C7A", "#2FD3E0"];
  function rgb(hex) { return [parseInt(hex.substr(1, 2), 16) / 255, parseInt(hex.substr(3, 2), 16) / 255, parseInt(hex.substr(5, 2), 16) / 255]; }
  function keys(prop, list) { for (var i = 0; i < list.length; i++) prop.setValueAtTime(list[i][0], list[i][1]); }
  function linearize(layer) {
    var props = [layer.transform.position, layer.transform.scale, layer.transform.opacity, layer.transform.rotation];
    for (var p = 0; p < props.length; p++) for (var k = 1; k <= props[p].numKeys; k++) props[p].setInterpolationTypeAtKey(k, KeyframeInterpolationType.LINEAR, KeyframeInterpolationType.LINEAR);
  }
  function newProject() { if (app.project) app.project.close(CloseOptions.DO_NOT_SAVE_CHANGES); app.newProject(); }
  function shape(comp, kind, color, w, h, name) {
    var L = comp.layers.addShape(); L.name = name;
    var g = L.property("ADBE Root Vectors Group").addProperty("ADBE Vector Group");
    var c = g.property("ADBE Vectors Group");
    var s = kind === "ellipse" ? c.addProperty("ADBE Vector Shape - Ellipse") : kind === "star" ? c.addProperty("ADBE Vector Shape - Star") : c.addProperty("ADBE Vector Shape - Rect");
    if (kind === "star") { s.property("ADBE Vector Star Type").setValue(1); s.property("ADBE Vector Star Points").setValue(5); s.property("ADBE Vector Star Outer Radius").setValue(w / 2); s.property("ADBE Vector Star Inner Radius").setValue(w / 4); }
    else { s.property(kind === "ellipse" ? "ADBE Vector Ellipse Size" : "ADBE Vector Rect Size").setValue([w, h]); if (kind === "rect") s.property("ADBE Vector Rect Roundness").setValue(18); }
    c.addProperty("ADBE Vector Graphic - Fill").property("ADBE Vector Fill Color").setValue(rgb(color));
    return L;
  }
  function text(comp, str, color, name) {
    var L = comp.layers.addText(str); L.name = name;
    var td = L.property("ADBE Text Properties").property("ADBE Text Document"), d = td.value;
    d.resetCharStyle(); d.fontSize = 64; d.font = "Helvetica-Bold"; d.fillColor = rgb(color); d.applyFill = true; d.justification = ParagraphJustification.CENTER_JUSTIFY; d.tracking = 0;
    td.setValue(d);
    // Centre the anchor like Powermove's centred text box.
    var r = L.sourceRectAtTime(0, false); L.transform.anchorPoint.setValue([r.left + r.width / 2, r.top + r.height / 2]);
    return L;
  }
  function element(comp, i, kind, cw, ch) {
    var col = i % 6, row = Math.floor(i / 6) % 4, x = cw * (.12 + col * .152), y = ch * (.18 + row * .21);
    var t0 = (i % 10) * .25, color = PAL[i % PAL.length];
    var L = kind === "text" ? text(comp, "MOTION " + (i + 1), color, "Text " + (i + 1))
      : shape(comp, i % 3 === 0 ? "ellipse" : i % 3 === 1 ? "rect" : "star", color, 160, 160, "Shape " + (i + 1));
    L.transform.position.dimensionsSeparated = true;
    keys(L.transform.xPosition, [[t0, x - 220], [t0 + 2, x], [8, x + 60], [10, x - 40]]);
    keys(L.transform.yPosition, [[t0, y + 90], [t0 + 2, y], [10, y - 30]]);
    keys(L.transform.scale, [[t0, [40, 40]], [t0 + 1.5, [110, 110]], [t0 + 3, [100, 100]], [10, [85, 85]]]);
    keys(L.transform.opacity, [[t0, 0], [t0 + 1, 100], [9, 100], [10, 20]]);
    var ps = [L.transform.xPosition, L.transform.yPosition, L.transform.scale, L.transform.opacity];
    for (var p = 0; p < ps.length; p++) for (var k = 1; k <= ps[p].numKeys; k++) ps[p].setInterpolationTypeAtKey(k, KeyframeInterpolationType.LINEAR, KeyframeInterpolationType.LINEAR);
    return L;
  }
  function gaussian(L, amount) { var e = L.property("ADBE Effect Parade").addProperty("ADBE Gaussian Blur 2"); e.property("ADBE Gaussian Blur 2-0001").setValue(amount); e.property("ADBE Gaussian Blur 2-0003").setValue(1); return e; }
  function save(name) { app.project.save(File(OUT + "/" + name + ".aep")); }

  // Scene A
  newProject();
  var A = app.project.items.addComp("Bench A motion graphics", W, H, 1, DUR, FPS); A.bgColor = rgb("#0C0D12");
  var pre = [app.project.items.addComp("Precomp 1", 960, 540, 1, DUR, FPS), app.project.items.addComp("Precomp 2", 960, 540, 1, DUR, FPS)];
  var n = 0, rootEls = [];
  for (var i = 0; i < 24; i++) rootEls.push(element(A, n++, i % 2 ? "text" : "shape", W, H));
  for (var c = 0; c < 2; c++) for (var j = 0; j < 8; j++) element(pre[c], n++, j % 2 ? "text" : "shape", 960, 540);
  for (var c2 = 0; c2 < 2; c2++) {
    var P = A.layers.add(pre[c2]); P.transform.anchorPoint.setValue([0, 0]);
    P.transform.position.setValuesAtTimes([0, 10], [[c2 ? 900 : 60, c2 ? 500 : 40], [c2 ? 960 : 0, c2 ? 500 : 40]]);
    keys(P.transform.scale, [[0, [90, 90]], [10, [100, 100]]]); keys(P.transform.opacity, [[0, 60], [2, 100]]); linearize(P);
  }
  // Powermove blurs the 4 bottom-most root elements (the first 4 created).
  for (var b = 0; b < 4; b++) gaussian(rootEls[b], 6 + (3 - b) * 4);
  save("scene-A");

  // Scene B
  newProject();
  var B = app.project.items.addComp("Bench B edit", W, H, 1, DUR, FPS); B.bgColor = [0, 0, 0];
  var EDIT = [["clip1.mp4", 0, 2.0, 0], ["clip2.mp4", 2.0, 3.7, 0], ["clip3.mp4", 3.2, 5.4, .5], ["clip4.mp4", 5.4, 7.0, 0], ["clip5.mp4", 6.5, 8.5, .5], ["clip6.mp4", 8.5, 10, 0]];
  for (var e = 0; e < EDIT.length; e++) {
    var it = app.project.importFile(new ImportOptions(File(MEDIA + "/" + EDIT[e][0])));
    var V = B.layers.add(it); V.name = "Clip " + (e + 1);
    V.startTime = EDIT[e][1] - .5; V.inPoint = EDIT[e][1]; V.outPoint = EDIT[e][2];
    if (EDIT[e][3]) keys(V.transform.opacity, [[EDIT[e][1], 0], [EDIT[e][1] + EDIT[e][3], 100]]);
  }
  var music = app.project.importFile(new ImportOptions(File(MEDIA + "/music.wav")));
  var M = B.layers.add(music); M.moveToEnd();
  save("scene-B");

  // Scene C
  newProject();
  var C = app.project.items.addComp("Bench C effects stress", W, H, 1, DUR, FPS); C.bgColor = rgb("#101014");
  for (var s = 0; s < 10; s++) {
    var x = 230 + (s % 5) * 365, y = 330 + Math.floor(s / 5) * 420;
    var S = shape(C, s % 2 ? "ellipse" : "rect", PAL[s % PAL.length], 300, 300, "Solid " + (s + 1));
    S.property("ADBE Root Vectors Group").property(1).property("ADBE Vectors Group").property(1);
    S.transform.position.setValuesAtTimes([0, 10], [[x - 40, y], [x + 40, y]]);
    keys(S.transform.rotation, [[0, 0], [10, 90 + s * 9]]); linearize(S);
    var fx = S.property("ADBE Effect Parade");
    gaussian(S, 8);
    var g = fx.addProperty("ADBE Glo2"); g.property("ADBE Glo2-0002").setValue(40 / 100 * 100); g.property("ADBE Glo2-0003").setValue(40); g.property("ADBE Glo2-0004").setValue(1.5);
    var lv = fx.addProperty("ADBE Easy Levels2"); lv.property("ADBE Easy Levels2-0004").setValue(10 / 100); lv.property("ADBE Easy Levels2-0005").setValue(90 / 100); lv.property("ADBE Easy Levels2-0006").setValue(1.2);
  }
  var adj = C.layers.addSolid([1, 1, 1], "Grade", W, H, 1, DUR); adj.adjustmentLayer = true;
  var hs = adj.property("ADBE Effect Parade").addProperty("ADBE HUE SATURATION"); hs.property("ADBE HUE SATURATION-0005").setValue(30);
  var bc = adj.property("ADBE Effect Parade").addProperty("ADBE Brightness & Contrast 2"); bc.property("ADBE Brightness & Contrast 2-0002").setValue(20);
  save("scene-C");
  var log = File(OUT + "/build_ok.txt"); log.open("w"); log.writeln("ok"); log.close();
})();
