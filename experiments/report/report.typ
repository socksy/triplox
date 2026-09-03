// Triplox query experiments — report
#import "@preview/cetz:0.4.2": canvas, draw

#let ink = rgb("#1f2421")
#let ink2 = rgb("#5d635f")
#let muted = rgb("#8d928e")
#let rule = rgb("#cfd4cf")
#let rule-soft = rgb("#e4e8e4")
#let accent = rgb("#2e5f4e")
#let accent-soft = rgb("#e3ece7")
#let accent-mid = rgb("#9dbfb1")
#let rust = rgb("#a9472f")
#let rust-soft = rgb("#f2e2dc")
#let paper = rgb("#fbfbf9")

#set page(paper: "a4", margin: (x: 22mm, top: 22mm, bottom: 24mm), fill: paper,
  footer: context [
    #set text(font: "Source Sans 3", size: 8pt, fill: muted)
    #grid(columns: (1fr, auto), [Triplox query experiments · 3 September 2026], [#counter(page).display()])
  ])
#set text(font: "Source Serif 4", size: 10pt, fill: ink)
#set par(justify: true, leading: 0.62em, spacing: 0.9em)
#show raw: set text(font: "Source Code Pro", size: 8.4pt)
#show raw.where(block: true): it => block(fill: white, stroke: 0.5pt + rule, inset: 7pt, radius: 1pt, width: 100%, it)
#show heading: set text(font: "Source Sans 3", weight: 600, fill: ink)
#show heading.where(level: 1): it => { v(0.2em); text(size: 16pt, it); v(0.35em); line(length: 100%, stroke: 0.5pt + rule); v(0.5em) }
#show heading.where(level: 2): it => { v(1.1em); text(size: 11.5pt, it); v(0.3em) }
#show heading.where(level: 3): it => { v(0.5em); text(size: 10pt, it); v(0.1em) }
#set heading(numbering: "1.1")
#set table(stroke: none, inset: (x: 5pt, y: 3.5pt))
#show table.cell.where(y: 0): set text(font: "Source Sans 3", size: 8pt, weight: 600, fill: ink2)
#show table: set text(size: 8.8pt)
#show table: set par(justify: false)
#show figure.caption: set text(size: 8.5pt, fill: ink2)
#show figure.caption: set par(justify: false)
#show figure: set block(above: 1.1em, below: 1.2em)
#set enum(spacing: 0.7em)
#set list(spacing: 0.7em)

#let sans(body, size: 9pt, fill: ink2, weight: 400) = text(font: "Source Sans 3", size: size, fill: fill, weight: weight, body)
#let mono(body) = text(font: "Source Code Pro", size: 8.4pt, body)
#let toggle(body) = text(font: "Source Code Pro", size: 8pt, fill: muted, body)
#let num(v) = if v >= 100 { str(calc.round(v)) } else if v >= 10 { str(calc.round(v, digits: 1)) } else { str(calc.round(v, digits: 2)) }
#let ratio-text(r) = {
  let s = str(calc.round(r, digits: 2)) + "×"
  if r < 0.85 { text(fill: accent, s) } else if r > 1.15 { text(fill: rust, s) } else { s }
}
#let hl = table.hline(stroke: 0.5pt + rule-soft)
#let hlh = table.hline(stroke: 0.6pt + rule)
#let term(body) = text(style: "italic", body)

#let data = json("results.json")
#let order = ("triangles", "two_hop_count", "three_hop_count", "out_degree", "in_degree_top",
  "neighbors_of_42", "weight_filter", "weight_sum", "heavy_neighbors", "label_lookup")

#let ab-table(key) = {
  let d = data.at(key)
  let extra = d.keys().filter(q => q not in order)
  let rows = ()
  for q in order + extra {
    if q in d {
      let r = d.at(q)
      rows.push(mono(q)); rows.push(align(right, str(r.rows))); rows.push(align(right, num(r.off)))
      rows.push(align(right, num(r.on))); rows.push(align(right, ratio-text(r.on / r.off)))
    }
  }
  table(columns: (1fr, auto, auto, auto, auto), align: left,
    table.header([query], align(right)[rows], align(right)[off ms], align(right)[on ms], align(right)[on / off]), hlh,
    ..rows.chunks(5).map(r => r + (hl,)).flatten())
}


#let stats = json("stats.json")
#let fmt2(x) = { let c = calc.round(x * 100); let w = calc.floor(c / 100); let f = c - w * 100; str(w) + "." + (if f < 10 { "0" } else { "" }) + str(f) }
#let thousands(n) = { let t = str(n); if t.len() <= 3 { t } else { thousands(calc.floor(n / 1000)) + "," + { let r = str(calc.rem(n, 1000)); "0" * (3 - r.len()) + r } } }
#let sig2(v) = { let m = calc.pow(10, calc.floor(calc.log(v, base: 10)) - 1); int(calc.round(v / m) * m) }
#let fmtx(r) = { let v = 1 / r; if v >= 100 { thousands(sig2(v)) } else if v >= 10 { str(calc.round(v)) } else { let c = calc.round(v * 10); str(calc.floor(c / 10)) + "." + str(int(c - calc.floor(c / 10) * 10)) } }
#let pfmt(p) = if p < 0.001 { "<0.001" } else { str(calc.round(p, digits: 3)) }
#let ci(v) = "[" + num(v.at(0)) + ", " + num(v.at(1)) + "]"
#let rci(v) = "[" + fmt2(v.at(0)) + ", " + fmt2(v.at(1)) + "]"
#let sig(r) = r.ratio_ci95.at(1) < 1 or r.ratio_ci95.at(0) > 1
#let ratio-cell(r) = {
  let t = fmt2(r.ratio_vs_off) + "×"
  let col = if not sig(r) { ink2 } else if r.ratio_vs_off < 1 { accent } else { rust }
  text(fill: col, t)
}
// Two-arm table with medians, bootstrap CIs, ratio CI and Mann-Whitney p
#let stats-table(key) = {
  if key not in stats { if key in data { ab-table(key) } else { block(width: 100%, stroke: (paint: rule, thickness: 0.5pt, dash: "dashed"), inset: 8pt, sans(size: 8pt, fill: accent, weight: 600)[MEASUREMENT PENDING]) } } else {
    let e = stats.at(key); let arms = e.arms; let base = arms.at(0)
    let qs = order.filter(q => q in e.queries) + e.queries.keys().filter(q => q not in order)
    if arms.len() == 2 {
      let on = arms.at(1)
      let rows = ()
      for q in qs {
        let r = e.queries.at(q)
        if on not in r { continue }
        let a = r.at(base); let b = r.at(on)
        rows.push(mono(q)); rows.push(align(right, str(a.n)))
        rows.push(align(right, num(a.median))); rows.push(align(right, sans(size: 7.5pt, ci(a.ci95_median))))
        rows.push(align(right, num(b.median))); rows.push(align(right, sans(size: 7.5pt, ci(b.ci95_median))))
        rows.push(align(right, ratio-cell(b))); rows.push(align(right, sans(size: 7.5pt, rci(b.ratio_ci95)))); rows.push(align(right, pfmt(b.p_mannwhitney)))
      }
      table(columns: (1fr, auto, auto, auto, auto, auto, auto, auto, auto), align: left,
        table.header([query], align(right)[n], align(right)[off ms], align(right)[95% CI], align(right)[on ms], align(right)[95% CI], align(right)[on / off], align(right)[95% CI], align(right)[p]), hlh,
        ..rows.chunks(9).map(r => r + (hl,)).flatten())
    } else {
      let others = arms.slice(1)
      let rows = ()
      for q in qs {
        let r = e.queries.at(q)
        rows.push(mono(q)); rows.push(align(right, num(r.at(base).median)))
        for arm in others {
          if arm in r { rows.push(align(right, ratio-cell(r.at(arm)) + " " + sans(size: 7pt, rci(r.at(arm).ratio_ci95)))) } else { rows.push([]) }
        }
      }
      let ncol = 2 + others.len()
      table(columns: (1fr,) + (auto,) * (ncol - 1), align: left,
        table.header([query], align(right)[off ms], ..others.map(a => align(right, a + " / off"))), hlh,
        ..rows.chunks(ncol).map(r => r + (hl,)).flatten())
    }
  }
}

// log-scale ratio bar chart
#let bw = 118pt
#let lmin = calc.log(0.02, base: 10)
#let lmax = calc.log(3, base: 10)
#let xf(v) = (calc.log(calc.max(0.02, calc.min(3, v)), base: 10) - lmin) / (lmax - lmin) * bw
#let ratio-bar(r) = {
  let x1 = xf(1.0); let xr = xf(r); let bad = r > 1
  box(width: bw, height: 9pt, {
    for t in (0.05, 0.1, 0.25, 0.5, 1, 2) {
      place(dx: xf(t), dy: -1pt, line(angle: 90deg, length: 11pt, stroke: (paint: if t == 1 { rule } else { rule-soft }, thickness: if t == 1 { 0.8pt } else { 0.4pt })))
    }
    place(dx: calc.min(x1, xr), dy: 1.5pt, rect(width: calc.max(1pt, calc.abs(xr - x1)), height: 6pt, fill: if bad { rust } else { accent }, stroke: none))
  })
}
#let axis-row() = box(width: bw, height: 8pt, {
  for t in (0.05, 0.1, 0.25, 0.5, 1, 2) {
    place(dx: xf(t) - 8pt, dy: 0pt, box(width: 16pt, align(center, sans(size: 6.5pt, fill: muted, str(t) + "×"))))
  }
})
#let panel(key, title, tog) = {
  let d = data.at(key)
  block(breakable: false, {
    sans(size: 9pt, fill: ink, weight: 600, title); linebreak(); toggle(tog); v(3pt)
    grid(columns: (auto, bw, auto), column-gutter: 6pt, row-gutter: 2.2pt, align: (right + horizon, left + horizon, left + horizon),
      ..order.map(q => {
        let r = d.at(q).on / d.at(q).off
        (sans(size: 7.4pt, q), ratio-bar(r), sans(size: 7pt, fill: ink2, str(calc.round(r, digits: 2))))
      }).flatten(),
      [], axis-row(), [])
  })
}

// ---------- cetz helpers (1 unit = 1cm) ----------
#let ctext(body, size: 7.6pt, fill: ink) = text(font: "Source Code Pro", size: size, fill: fill, body)
#let stext(body, size: 7.6pt, fill: ink2, weight: 400) = text(font: "Source Sans 3", size: size, fill: fill, weight: weight, body)
#let cell(x, y, w, h, body, fill: white, stroke: rule, size: 7.6pt) = {
  draw.rect((x, y), (x + w, y + h), fill: fill, stroke: 0.5pt + stroke)
  draw.content((x + w / 2, y + h / 2), ctext(body, size: size))
}
#let strip(x, y, w, h, items, fills: none, stroke: rule, size: 7.6pt) = {
  for (i, it) in items.enumerate() {
    let f = if fills == none { white } else { fills.at(i) }
    cell(x + i * w, y, w, h, it, fill: f, stroke: stroke, size: size)
  }
}
#let label(x, y, body, anchor: "west", size: 7.6pt, fill: ink2) = draw.content((x, y), anchor: anchor, stext(body, size: size, fill: fill))
#let arrow(a, b, stroke: ink2) = draw.line(a, b, stroke: 0.7pt + stroke, mark: (end: "straight", scale: 0.6))
#let node(x, y, name, fill: white) = {
  draw.circle((x, y), radius: 0.26, fill: fill, stroke: 0.6pt + ink2)
  draw.content((x, y), ctext(name, size: 7.2pt))
}
#let edge(a, b, stroke: accent) = {
  let (ax, ay) = a; let (bx, by) = b
  let dx = bx - ax; let dy = by - ay; let l = calc.sqrt(dx * dx + dy * dy)
  let ux = dx / l; let uy = dy / l
  draw.line((ax + ux * 0.29, ay + uy * 0.29), (bx - ux * 0.33, by - uy * 0.33), stroke: 0.8pt + stroke, mark: (end: "straight", scale: 0.55, fill: stroke))
}
#let example-graph(ox, oy, hl: ()) = {
  let P = (a: (ox + 0.0, oy + 1.2), b: (ox + 1.4, oy + 2.1), c: (ox + 2.8, oy + 1.2), d: (ox + 1.4, oy + 0.3), e: (ox - 1.4, oy + 2.0))
  for (u, v) in (("a", "b"), ("a", "c"), ("b", "c"), ("b", "d"), ("c", "d"), ("d", "a"), ("e", "a")) {
    edge(P.at(u), P.at(v), stroke: if (u, v) in hl { rust } else { accent })
  }
  for (k, p) in P { node(p.at(0), p.at(1), k + "=" + str(("a": 1, "b": 2, "c": 3, "d": 4, "e": 5).at(k))) }
}

#let fig(body, caption) = figure(block(width: 100%, inset: (y: 2pt), align(center, body)), caption: caption)

// ---------- ratio charts ----------
#let arm-color = (on: accent, batched: rgb("#3d8a68"), segments: rgb("#4a6d9c"), columnar: rgb("#7fa3cc"), adj: rgb("#c48a2a"), algebra: rgb("#8a4b2a"), zone: rgb("#8a5f9e"), all: ink)
#let panel-bg = rgb("#eceeea")
#let bar-good = rgb("#5b9a7c")
#let bar-bad = rgb("#c8705a")
#let bar-none = rgb("#c3c8c1")
#let tick-cands = (0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1, 2, 5, 10)
#let arm-names = (batched: "vectorized join", segments: "datoms per key", columnar: "columnar segments", adj: "adjacency matrices", algebra: "matrix algebra", zone: "zone maps", all: "all on", on: "on")
#let arm-overrides = ("combo-full-algebra": (all: "join + columnar + matrices", algebra: "the same + algebra"), "sparse-algebra": (adj: "matrices only", algebra: "matrices + algebra"))
#let arm-name(a, key: none) = if key != none and key in arm-overrides and a in arm-overrides.at(key) { arm-overrides.at(key).at(a) } else { arm-names.at(a) }
// rows: array of (query, (arm: (ratio, lo, hi, significant, off-ms, on-ms)))
#let ratio-chart(rows, arms) = {
  let vals = ()
  for (n, d) in rows { for (a, v) in d { vals.push(v.at(0)); vals.push(v.at(1)); vals.push(v.at(2)) } }
  let vmin = calc.min(..vals); let vmax = calc.max(..vals)
  let below = tick-cands.filter(t => t <= vmin * 0.97); let above = tick-cands.filter(t => t >= vmax * 1.03)
  let lmin = if below.len() == 0 { 0.01 } else { below.last() }
  let lmax = if above.len() == 0 { 10 } else { above.first() }
  let x0 = 3.1; let x1 = 10.6
  let lx(v) = x0 + (calc.log(v, base: 10) - calc.log(lmin, base: 10)) / (calc.log(lmax, base: 10) - calc.log(lmin, base: 10)) * (x1 - x0)
  let k = arms.len()
  let rh = if k == 1 { 0.5 } else { 0.3 + 0.17 * k }
  let top = 0.0
  canvas(length: 1cm, {
    // panel, grid and tick labels
    draw.rect((x0 - 0.15, top + 0.12), (x1 + 0.15, top - rows.len() * rh - 0.08), fill: panel-bg, stroke: none)
    for t in tick-cands.filter(t => t >= lmin and t <= lmax) {
      let one = t == 1
      draw.line((lx(t), top + 0.12), (lx(t), top - rows.len() * rh - 0.08), stroke: (paint: if one { ink } else { white }, thickness: if one { 0.8pt } else { 1pt }))
      draw.content((lx(t), top + 0.35), stext(str(t) + "×", size: 6.6pt, fill: if one { ink } else { ink2 }))
    }
    draw.content((x0, top - rows.len() * rh - 0.3), anchor: "west", stext([← faster than off], size: 6.6pt, fill: muted))
    draw.content((x1, top - rows.len() * rh - 0.3), anchor: "east", stext([slower than off →], size: 6.6pt, fill: muted))
    if k > 1 {
      let lxp = 0.0
      for (i, a) in arms.enumerate() {
        draw.circle((lxp + 0.1, top + 0.85), radius: 0.09, fill: arm-color.at(a), stroke: none)
        draw.content((lxp + 0.28, top + 0.85), anchor: "west", stext(arm-name(a), size: 7pt, fill: ink))
        lxp = lxp + 0.5 + 0.13 * arm-name(a).len()
      }
      draw.content((x1 + 0.2, top + 0.85), anchor: "west", stext([times faster than off, in arm colour], size: 6.6pt, fill: muted))
    }
    for (ri, (name, d)) in rows.enumerate() {
      let y = top - (ri + 0.5) * rh
      draw.content((x0 - 0.2, y), anchor: "east", ctext(name, size: 7.2pt))
      let off = none
      for (i, a) in arms.enumerate() {
        if a not in d { continue }
        let (r, lo, hi, sg, o, n) = d.at(a)
        off = o
        let col = if not sg { bar-none } else if k > 1 { arm-color.at(a) } else if r < 1 { bar-good } else { bar-bad }
        let tcol = if not sg { muted } else if k > 1 { arm-color.at(a) } else if r < 1 { accent } else { rust }
        let dy = (k - 1) / 2 * 0.17 - i * 0.17
        let rr = calc.max(lmin, calc.min(lmax, r)); let llo = calc.max(lmin, lo); let lhi = calc.min(lmax, hi)
        if k == 1 {
          draw.rect((calc.min(lx(1), lx(rr)), y - 0.15), (calc.max(lx(1), lx(rr)), y + 0.15), fill: col, stroke: none)
          draw.line((lx(llo), y), (lx(lhi), y), stroke: 1pt + ink)
          draw.line((lx(llo), y - 0.12), (lx(llo), y + 0.12), stroke: 1pt + ink)
          draw.line((lx(lhi), y - 0.12), (lx(lhi), y + 0.12), stroke: 1pt + ink)
          draw.content((x1 + 0.3, y), anchor: "west", stext(fmt2(r) + "×", size: 7.2pt, fill: tcol))
          draw.content((x1 + 1.1, y), anchor: "west", stext(num(o) + " → " + num(n) + " ms", size: 6.8pt, fill: ink2))
        } else {
          draw.line((lx(llo), y + dy), (lx(lhi), y + dy), stroke: 0.9pt + tcol)
          draw.circle((lx(rr), y + dy), radius: 0.09, fill: if sg { tcol } else { white }, stroke: 0.7pt + tcol)
          draw.content((x1 + 0.3 + i * 0.85, y), anchor: "west", stext(fmt2(r), size: 6.8pt, fill: tcol))
        }
      }
      if k > 1 and off != none { draw.content((x1 + 0.3 + k * 0.85, y), anchor: "west", stext(num(off) + " ms", size: 6.8pt, fill: ink2)) }
    }
  })
}
#let chart-caption = [Median time with the toggle on, divided by median time with it off, per query. The axis is logarithmic: a bar reaching 0.1× means ten times faster. Bars to the left of 1× are gains, bars to the right are losses. The black whisker is the 95% bootstrap interval of the ratio. A grey bar means the interval includes 1× and the report treats the change as no effect. The medians in milliseconds stand at the right.]
#let pending = block(width: 100%, stroke: (paint: rule, thickness: 0.5pt, dash: "dashed"), inset: 8pt, sans(size: 8pt, fill: accent, weight: 600)[MEASUREMENT PENDING])
// ---------- dumbbell chart on an absolute log time axis, with raw samples ----------
#let ms-label(t) = if t < 1 { str(t) + " ms" } else if t >= 1000 { str(calc.round(t / 1000)) + " s" } else { str(t) + " ms" }
// rows: array of (query, ratio-text-or-none, ratio-colour, (arm: (median, samples)))
#let time-chart(rows, arms, key: none) = {
  let vals = ()
  for (n, rt, rc, d) in rows { for (a, v) in d { vals.push(v.at(0)); for x in v.at(1) { vals.push(x) } } }
  let vmin = calc.max(0.005, calc.min(..vals)); let vmax = calc.max(..vals)
  let lmin = calc.pow(10, calc.floor(calc.log(vmin, base: 10)))
  let lmax = calc.pow(10, calc.ceil(calc.log(vmax, base: 10)))
  let x0 = 3.1; let x1 = 11.2
  let lx(v) = x0 + (calc.log(calc.max(v, lmin), base: 10) - calc.log(lmin, base: 10)) / (calc.log(lmax, base: 10) - calc.log(lmin, base: 10)) * (x1 - x0)
  let k = arms.len()
  let rh = if k == 2 { 0.52 } else { 0.22 + 0.17 * k }
  let top = 0.0
  let bottom = top - rows.len() * rh - 0.08
  canvas(length: 1cm, {
    draw.rect((x0 - 0.15, top + 0.12), (x1 + 0.15, bottom), fill: panel-bg, stroke: none)
    let t = lmin
    while t <= lmax {
      draw.line((lx(t), top + 0.12), (lx(t), bottom), stroke: 1pt + white)
      draw.content((lx(t), bottom - 0.2), stext(ms-label(t), size: 6.4pt, fill: ink2))
      for m in (2, 5) { if t * m < lmax { draw.line((lx(t * m), top + 0.12), (lx(t * m), bottom), stroke: 0.5pt + white) } }
      t = t * 10
    }
    draw.content(((x0 + x1) / 2, bottom - 0.5), stext([time per query, logarithmic], size: 6.8pt, fill: ink2))
    let lxp = x0
    for (i, a) in arms.enumerate() {
      let col = if i == 0 { ink2 } else { arm-color.at(a) }
      draw.circle((lxp + 0.1, top + 0.4), radius: 0.07, fill: col, stroke: none)
      draw.content((lxp + 0.26, top + 0.4), anchor: "west", stext(if i == 0 { "off" } else { arm-name(a, key: key) }, size: 6.8pt, fill: ink))
      lxp = lxp + 0.55 + 0.12 * (if i == 0 { 3 } else { arm-name(a, key: key).len() })
    }
    if k == 2 { draw.content((x1 + 0.15, top + 0.4), anchor: "east", stext([small dots: the 20 samples · large: median], size: 6.4pt, fill: muted)) }
    for (ri, (name, rt, rc, d)) in rows.enumerate() {
      let y = top - (ri + 0.5) * rh
      draw.content((x0 - 0.25, y), anchor: "east", ctext(name, size: 7.2pt))
      let base = arms.at(0)
      let bm = if base in d { d.at(base).at(0) } else { none }
      for (i, a) in arms.enumerate() {
        if a not in d { continue }
        let (m, xs) = d.at(a)
        let col = if i == 0 { ink2 } else { arm-color.at(a) }
        let dy = if k == 2 { (if i == 0 { 0.1 } else { -0.1 }) } else { (k - 1) / 2 * 0.16 - i * 0.16 }
        for x in xs { draw.circle((lx(x), y + dy), radius: 0.04, fill: col.transparentize(55%), stroke: none) }
        if i > 0 and bm != none { draw.line((lx(bm), y + dy), (lx(m), y + dy), stroke: 0.8pt + col) }
        draw.circle((lx(m), y + dy), radius: 0.1, fill: if i == 0 { white } else { col }, stroke: 0.8pt + col)
      }
      if rt != none { draw.content((x1 + 0.3, y), anchor: "west", stext(rt, size: 6.8pt, fill: rc)) }
    }
  })
}
#let ratio-words(r, sg) = if not sg { "no effect" } else if r < 1 { (if 1 / r >= 10 { str(calc.round(1 / r, digits: 1)) } else { fmt2(1 / r) }) + "× faster" } else { fmt2(r) + "× slower" }
#let time-chart-caption = [Small dots are the 20 samples of each arm, large markers the medians. "No effect": the 95% interval of the ratio includes 1.]
#let stats-chart(key) = {
  if key in stats {
    let e = stats.at(key); let arms = e.arms; let base = arms.at(0); let others = arms.slice(1)
    let qs = order.filter(q => q in e.queries) + e.queries.keys().filter(q => q not in order)
    let rows = ()
    for q in qs {
      let r = e.queries.at(q); let d = (:)
      for a in arms { if a in r { d.insert(a, (r.at(a).median, r.at(a).at("samples", default: ()))) } }
      let rt = none; let rc = ink2
      if others.len() == 1 and others.at(0) in r and ("ratio_vs_" + base) in r.at(others.at(0)) {
        let b = r.at(others.at(0)); rt = ratio-words(b.ratio_vs_off, sig(b)); rc = if not sig(b) { muted } else if b.ratio_vs_off < 1 { accent } else { rust }
      } else if others.len() > 1 {
        let parts = ()
        for a in others { if a in r and ("ratio_vs_" + base) in r.at(a) { let b = r.at(a); parts.push(text(fill: if sig(b) { arm-color.at(a) } else { muted }, fmtx(b.ratio_vs_off))) } }
        rt = parts.join(" · "); rc = ink2
      }
      rows.push((q, rt, rc, d))
    }
    figure(block(width: 100%, inset: (y: 2pt), align(center, time-chart(rows, arms, key: key))), caption: if others.len() == 1 { time-chart-caption } else { [Small dots are the 20 samples of each arm, large markers the medians. Right: times faster than off per arm, grey when the 95% interval includes 1.] })
  } else if key in data {
    let d = data.at(key)
    let rows = order.filter(q => q in d).map(q => { let r = d.at(q).on / d.at(q).off; (q, ratio-words(r, r < 0.85 or r > 1.15), if r < 0.85 { accent } else if r > 1.15 { rust } else { muted }, (off: (d.at(q).off, ()), on: (d.at(q).on, ()))) })
    figure(block(width: 100%, inset: (y: 2pt), align(center, time-chart(rows, ("off", "on")))), caption: [Median time per query, toggle off and on, from the first A/B run. No samples: the statistics pass replaces this chart.])
  } else { pending }
}
#let verdict(..rows) = table(columns: (auto, 1fr), align: (left, left), ..rows.pos().chunks(2).map(r => (sans(weight: 600, fill: ink2, r.at(0)), text(font: "Source Serif 4", size: 8.8pt, weight: 400, fill: ink, r.at(1)), hl)).flatten())

#let q-color = (triangles: rgb("#2e5f4e"), two_hop_count: rgb("#2a7f8f"), three_hop_count: rgb("#1f3a5f"), out_degree: rgb("#4a6d9c"), in_degree_top: rgb("#7f9cc9"), neighbors_of_42: rgb("#c48a2a"), weight_filter: rgb("#8a5f9e"), weight_sum: rgb("#b56576"), heavy_neighbors: rgb("#c8705a"), label_lookup: rgb("#1f2421"))
#let config-medians() = {
  let src = (("off", none, "off: current engine"), ("batched", ("vectorized-join", "on"), "vectorized join (ch. 2)"), ("segments", ("datom-segments", "on"), "datoms per key (ch. 3)"), ("columnar", ("columnar-storage", "on"), "columnar segments (ch. 4)"), ("adj", ("sparse-matrix", "on"), "adjacency matrices (ch. 5)"), ("zone", ("zone-maps", "on"), "zone maps (ch. 6)"), ("all", ("combo-full", "all"), "full stack (ch. 7)"))
  let out = ()
  for (a, kv, title) in src {
    let d = (:)
    for q in order {
      if kv == none {
        let offs = ("vectorized-join", "datom-segments", "columnar-storage", "sparse-matrix", "zone-maps", "combo-full").filter(k => k in stats and q in stats.at(k).queries).map(k => stats.at(k).queries.at(q).off.median).sorted()
        if offs.len() > 0 { d.insert(q, offs.at(calc.floor(offs.len() / 2))) }
      } else if kv.at(0) in stats and q in stats.at(kv.at(0)).queries and kv.at(1) in stats.at(kv.at(0)).queries.at(q) { d.insert(q, stats.at(kv.at(0)).queries.at(q).at(kv.at(1)).median) }
    }
    if d.len() > 0 { out.push((title, d)) }
  }
  out
}
#let stacked-workload(configs, qs, x0, y0, width, unit-label) = {
  let total(d) = qs.filter(q => q in d).map(q => d.at(q)).sum(default: 0)
  let tmax = calc.max(..configs.map(c => total(c.at(1))))
  let sx(v) = x0 + v / tmax * width
  let bh = 0.36; let gap = 0.16
  // grid
  let step = if tmax > 4000 { 1000 } else if tmax > 1500 { 250 } else if tmax > 600 { 100 } else { 50 }
  let t = 0
  while t <= tmax { draw.line((sx(t), y0 + 0.15), (sx(t), y0 - configs.len() * (bh + gap) + 0.05), stroke: 0.5pt + rule-soft); draw.content((sx(t), y0 - configs.len() * (bh + gap) - 0.12), stext(if step >= 1000 { str(calc.round(t / 1000, digits: 1)) + " s" } else { str(t) + " ms" }, size: 6.2pt, fill: ink2)); t = t + step }
  for (i, (title, d)) in configs.enumerate() {
    let y = y0 - i * (bh + gap) - bh
    draw.content((x0 - 0.15, y + bh / 2), anchor: "east", stext(title, size: 6.8pt, fill: ink))
    let x = 0
    for q in qs {
      if q not in d { continue }
      let v = d.at(q)
      draw.rect((sx(x), y), (sx(x + v), y + bh), fill: q-color.at(q), stroke: 0.4pt + white)
      x = x + v
    }
    draw.content((sx(x) + 0.1, y + bh / 2), anchor: "west", stext(if x >= 1000 { str(calc.round(x / 1000, digits: 2)) + " s" } else { str(calc.round(x)) + " ms" }, size: 6.6pt, fill: ink2))
  }
  draw.content((x0 + width / 2, y0 - configs.len() * (bh + gap) - 0.4), stext(unit-label, size: 6.6pt, fill: ink2))
}
#let overview-stacked() = {
  let configs = config-medians()
  canvas(length: 1cm, {
    let x0 = 4.2; let w = 8.6
    draw.content((0, 0.55), anchor: "west", stext([Total time to run the ten queries once, linear scale], size: 7.8pt, fill: ink, weight: 600))
    stacked-workload(configs, order, x0, 0.0, w, [three_hop_count is the pale block: it is 85% of the workload, and the nine other queries share the rest])
    let y2 = -configs.len() * 0.52 - 1.3
    draw.content((0, y2 + 0.55), anchor: "west", stext([The same, without three_hop_count], size: 7.8pt, fill: ink, weight: 600))
    stacked-workload(configs, order.filter(q => q != "three_hop_count"), x0, y2, w, [label_lookup is 0.02 ms: on this scale it has no width at all])
    // legend
    let y3 = y2 - configs.len() * 0.52 - 1.15
    let lx = 0.0
    for q in order {
      draw.rect((lx, y3 - 0.09), (lx + 0.22, y3 + 0.09), fill: q-color.at(q), stroke: none)
      draw.content((lx + 0.3, y3), anchor: "west", stext(q, size: 6.2pt, fill: ink))
      lx = lx + 0.55 + 0.095 * q.len()
      if lx > 12.5 { lx = 0.0; y3 = y3 - 0.3 }
    }
  })
}
#let config-stats() = {
  let src = (("batched", ("vectorized-join", "on"), "vectorized join", "ch. 2"), ("segments", ("datom-segments", "on"), "datoms per key", "ch. 3"), ("columnar", ("columnar-storage", "on"), "columnar segments", "ch. 4"), ("adj", ("sparse-matrix", "on"), "adjacency matrices", "ch. 5"), ("zone", ("zone-maps", "on"), "zone maps", "ch. 6"), ("algebra", ("sparse-algebra", "algebra"), "matrix algebra", "ch. 5"), ("all", ("combo-full-algebra", "algebra"), "all four", "ch. 7"))
  let out = ()
  for (a, kv, title, ch) in src {
    if kv.at(0) not in stats { continue }
    let d = (:); let sg = (:); let off = (:)
    for q in order {
      let qs = stats.at(kv.at(0)).queries
      if q in qs and kv.at(1) in qs.at(q) { let b = qs.at(q).at(kv.at(1)); d.insert(q, b.median); sg.insert(q, sig(b)); off.insert(q, qs.at(q).off.median) }
    }
    out.push((title, ch, d, sg, off))
  }
  out
}
#let nudge(items, minsep) = {
  // items: array of (key, y) sorted ascending; returns array of display y
  let ys = (); let prev = -100
  for (k, y) in items { let yy = calc.max(y, prev + minsep); ys.push(yy); prev = yy }
  ys
}
#let slopegraphs() = {
  let techs = config-stats()
  let off = techs.at(0).at(4)
  let H = 8.4
  let ly(v) = (calc.log(calc.max(v, 0.01), base: 10) + 2) / 6 * H
  let x0 = 3.7; let pw = 1.85; let span = 1.1
  canvas(length: 1cm, {
    // scale marks, once, at the far left
    for (v, l) in ((0.01, "10 µs"), (0.1, "0.1 ms"), (1, "1 ms"), (10, "10 ms"), (100, "100 ms"), (1000, "1 s"), (10000, "10 s")) {
      draw.content((0.85, ly(v)), anchor: "east", stext(l, size: 5.8pt, fill: muted))
    }
    draw.line((0.95, ly(0.01)), (0.95, ly(10000)), stroke: 0.3pt + rule)
    // query labels at their off height
    let items = order.filter(q => q in off).map(q => (q, ly(off.at(q)))).sorted(key: it => it.at(1))
    let ys = nudge(items, 0.26)
    for (i, (q, y)) in items.enumerate() {
      draw.content((x0 - 0.25, ys.at(i)), anchor: "east", ctext(q, size: 6.2pt, fill: q-color.at(q)) + stext(" " + num(off.at(q)), size: 5.8pt, fill: muted))
      if calc.abs(ys.at(i) - y) > 0.01 { draw.line((x0 - 0.22, ys.at(i)), (x0 - 0.05, y), stroke: 0.3pt + rule) }
    }
    draw.content((x0 - 0.25, H + 0.55), anchor: "east", stext([query, median ms with everything off], size: 6.2pt, fill: ink2))
    for (pi, (title, ch, d, sg, o)) in techs.enumerate() {
      let xa = x0 + pi * pw; let xb = xa + span
      draw.content(((xa + xb) / 2, H + 0.55), stext(title, size: 5.9pt, fill: ink))
      draw.content(((xa + xb) / 2, H + 0.3), stext(ch, size: 5.8pt, fill: muted))
      let labels = ()
      for q in order {
        if q not in d { continue }
        let r = d.at(q) / o.at(q)
        let col = if not sg.at(q) { rule } else { q-color.at(q) }
        draw.line((xa, ly(o.at(q))), (xb, ly(d.at(q))), stroke: (paint: col, thickness: if sg.at(q) { 0.8pt } else { 0.5pt }))
        draw.circle((xb, ly(d.at(q))), radius: 0.045, fill: col, stroke: none)
        if sg.at(q) { labels.push((if r < 1 { (if 1 / r >= 10 { str(calc.round(1 / r)) } else { str(calc.round(1 / r, digits: 1)) }) + "×" } else { str(calc.round(r, digits: 2)) + "× slower" }, ly(d.at(q)), col)) }
      }
      let sorted = labels.sorted(key: it => it.at(1))
      let lys = nudge(sorted.map(it => (it.at(0), it.at(1))), 0.24)
      for (i, (t, y, col)) in sorted.enumerate() { draw.content((xb + 0.1, lys.at(i)), anchor: "west", stext(t, size: 5.6pt, fill: col)) }
    }
  })
}
#let heat-rows = (("vectorized-join", "on", "Vectorized join (ch. 2)"), ("datom-segments", "on", "Datoms per key (ch. 3)"), ("columnar-storage", "on", "Columnar segments (ch. 4)"), ("sparse-matrix", "on", "Adjacency matrices (ch. 5)"), ("sparse-algebra", "algebra", "Matrix algebra (ch. 5)"), ("zone-maps", "on", "Zone maps (ch. 6)"), ("combo-vec-seg", "all", "Vectorized join + datoms per key (ch. 7)"), ("combo-vec-col", "all", "Vectorized join + columnar (ch. 7)"), ("combo-full-algebra", "all", "Vectorized + columnar + matrices (ch. 7)"), ("combo-full-algebra", "algebra", "The same plus matrix algebra (ch. 7)"))
#let short-q = (triangles: "triangles", two_hop_count: "two_hop", three_hop_count: "three_hop", out_degree: "out_degree", in_degree_top: "in_degree", neighbors_of_42: "neighbors", weight_filter: "weight_filter", weight_sum: "weight_sum", heavy_neighbors: "heavy_nbrs", label_lookup: "label_lookup")
#let heat-color(r, sg) = {
  if not sg { return panel-bg }
  let l = calc.abs(calc.log(r, base: 10))
  let p = calc.pow(calc.min(1, l / 1.3), 0.6)
  color.mix((if r < 1 { bar-good } else { bar-bad }, p * 100%), (white, (1 - p) * 100%))
}
#let heatmap() = canvas(length: 1cm, {
  let cw = 1.02; let ch = 0.5; let x0 = 5.4
  for (j, q) in order.enumerate() { draw.content((x0 + j * cw + cw / 2, 0.25), angle: 30deg, anchor: "west", stext(short-q.at(q), size: 6.4pt, fill: ink2)) }
  for (i, (key, arm, title)) in heat-rows.enumerate() {
    let y = -0.3 - i * ch
    draw.content((x0 - 0.2, y - ch / 2), anchor: "east", stext(title, size: 7pt, fill: ink))
    for (j, q) in order.enumerate() {
      let x = x0 + j * cw
      let v = none
      if key in stats and q in stats.at(key).queries and arm in stats.at(key).queries.at(q) {
        let b = stats.at(key).queries.at(q).at(arm); v = (b.ratio_vs_off, sig(b))
      } else if key in data and q in data.at(key) {
        let r = data.at(key).at(q).on / data.at(key).at(q).off; v = (r, r < 0.85 or r > 1.15)
      }
      if v == none { draw.rect((x, y - ch), (x + cw, y), fill: white, stroke: 0.6pt + white); draw.content((x + cw / 2, y - ch / 2), stext([–], size: 6.6pt, fill: muted)) } else {
        let (r, sg) = v
        draw.rect((x, y - ch), (x + cw, y), fill: heat-color(r, sg), stroke: 0.8pt + white)
        draw.content((x + cw / 2, y - ch / 2), stext(fmtx(r), size: 6.2pt, fill: if not sg { muted } else if calc.abs(calc.log(r, base: 10)) > 0.7 { white } else { ink }))
      }
    }
  }
  let yl = -0.3 - heat-rows.len() * ch - 0.45
  draw.content((x0, yl), anchor: "west", stext([cell = how many times faster than off. green: faster, darker is larger · red: below 1, slower · grey cell and grey number: no effect, the interval includes 1], size: 6.6pt, fill: ink2))
})


// ==================================================================
#sans(size: 8pt, fill: muted, upper[Experiment report · 3 September 2026])
#v(2pt)
#text(font: "Source Sans 3", size: 22pt, weight: 600)[Triplox Query Experiments]
#v(4pt)
#text(size: 11pt, fill: ink2)[Five storage and execution techniques from DuckDB, Datomic and RedisGraph. Each one is explained from first principles, built into Triplox end to end, and measured on one benchmark.]
#v(6pt)
#toggle[Branches: bench/harness, exp/vectorized-join, exp/datom-segments, exp/columnar-storage, exp/sparse-matrix, exp/zone-maps, exp/combo-vec-seg, exp/combo-vec-col, exp/combo-full. Nothing is pushed.]

#v(10pt)
Five techniques from other database engines were built into Triplox and measured on one benchmark: a vectorized join, many datoms per storage key, columnar segments, sparse adjacency matrices, and zone maps. The reference query throughout is the triangle query: find every three vertices a, b, c with edges a→b, b→c and a→c, the standard test of a graph join. A profile of it put 55% of its time in the join's row vectors and 28% in index iteration. The vectorized join removes the first cost. Many datoms per key remove most of the second. Adjacency matrices turn graph traversals into array operations and run the triangle query approximately 30 times faster. RedisGraph counts paths with one counter per vertex instead of listing them. Built the same way on top of the matrices, that takes the three-hop count from 6.8 s to under a millisecond. Zone maps only pay for time-travel queries. Three combinations of the techniques were then measured, and two latent defects in the current code were found on the way.

= The benchmark and the current engine

== The harness

The benchmark is a Cargo bench on the `bench/harness` branch, run as `cargo bench --bench datalog_bench`. It starts an in-memory node, loads a seeded random graph through the normal transaction path, and times ten Datalog queries through the public `db.query()` call. SlateDB runs on memory with no object store. The figures isolate engine and layout costs and say nothing about S3 latency. Against object storage, each key fetch becomes a block read, and the per-key costs in chapters 3 and 4 grow.

An environment variable guards each experiment, and one binary runs both arms of an A/B comparison. Every ratio in this report is on to off within one run of the same binary. The baseline column of the query table was measured earlier and separately, and no ratio uses it.

== Measurement

Every chart and table reports the median of 20 samples per arm and query. The samples come from four passes of five iterations, with the arms interleaved within each pass, on an otherwise idle machine. Confidence intervals on medians and on ratios of medians are bootstrap percentile intervals from 4000 resamples. The p-value comes from a two-sided Mann-Whitney U test between the samples of the two arms. A ratio is printed in colour only if its interval excludes 1. If the interval includes 1, the report treats the difference as no effect, whatever the point estimate. The first iteration of each query in a process is a cold run. It is included, and it widens the intervals on the sub-millisecond queries.

== Input

The benchmark graph is a directed G(n, p) graph with 2000 vertices and p = 0.01. It has 39,764 edges. Each vertex is an entity with four attributes. The edges are one cardinality-many reference attribute. Triplox stores the graph the way any application would store it, not as an adjacency list.

This is a narrow workload, and the results must be read with that in mind. The data is uniform and random, with integer keys and values, one string attribute, one reference attribute, and seven of the ten queries traverse that one attribute. A study of 250,000 real queries from Tableau Public found the opposite shape: half of all stored values are strings, most datasets hold under a thousand tuples with a long tail of huge ones, fewer than 5% of queries contain a join, and query cost sits in string handling, casts and other scalar expressions rather than in joins.#footnote[Vogelsgesang et al., "Get Real: How Benchmarks Fail to Represent the Real World", DBTest 2018.] This benchmark, like TPC-H, tests none of that. The vectorized join and the segment layouts cut costs that every query pays and should carry over. The adjacency matrices, the algebra and value-based zone maps are shaped for this data. String storage and expression evaluation were not tested at all, and a workload built from real Datomic use would be the fair test.

#table(columns: (auto, auto, auto, 1fr),
  table.header([attribute], [type], [values], [purpose]), hlh,
  mono[:g/id], [long, unique identity], [0 … 1999], [point lookups and lookup refs], hl,
  mono[:g/to], [ref, cardinality many], [about 20 targets per vertex], [all graph traversals], hl,
  mono[:g/weight], [long], [id mod 1000], [scalar range predicates and sums], hl,
  mono[:g/label], [string], ["label-#emph[id]"], [string equality lookup], hl)

== The running example

Two thousand vertices do not fit in a diagram. The rest of the report uses a five-vertex graph with the same schema. Entities are integers, and the letters are only labels. The seven edges contain two triangles: a→b→c closed by a→c, and b→c→d closed by b→d. Exactly two vertices have a weight above 950.

#fig(canvas(length: 1cm, {
  example-graph(1.6, 0.2)
  label(5.6, 2.5, [entity], size: 7.2pt, fill: ink); label(6.9, 2.5, [weight], size: 7.2pt, fill: ink); label(8.1, 2.5, [edges out], size: 7.2pt, fill: ink)
  let rows = (("a = 1", "7", "b, c"), ("b = 2", "42", "c, d"), ("c = 3", "960", "d"), ("d = 4", "3", "a"), ("e = 5", "999", "a"))
  for (i, r) in rows.enumerate() {
    draw.content((5.6, 2.05 - i * 0.38), anchor: "west", ctext(r.at(0)))
    draw.content((6.9, 2.05 - i * 0.38), anchor: "west", ctext(r.at(1)))
    draw.content((8.1, 2.05 - i * 0.38), anchor: "west", ctext(r.at(2)))
  }
}), [The example graph: seven `:g/to` edges over five entities. Transaction 1 asserted all edges except d→a, which transaction 2 asserted.])

== How Triplox stores the example

A transaction turns each fact into a #term[datom]: entity, attribute, value, transaction id, and a flag for assert or retract. Triplox writes every datom into six indexes in SlateDB, a log-structured key-value store. Each index holds the same datoms sorted in a different component order. Today each datom is one SlateDB key in each index. Graph queries use two of the indexes. AEV is sorted by attribute, entity, value and answers "what does entity #emph[x] point to". AVE is sorted by attribute, value, entity and answers "who points to #emph[y]".

#fig(canvas(length: 1cm, {
  label(0, 4.3, [datoms for :g/to, in transaction order], size: 7.6pt, fill: ink)
  let ds = ("[1 :g/to 2  tx1 +]", "[1 :g/to 3  tx1 +]", "[2 :g/to 3  tx1 +]", "[2 :g/to 4  tx1 +]", "[3 :g/to 4  tx1 +]", "[5 :g/to 1  tx1 +]", "[4 :g/to 1  tx2 +]")
  for (i, d) in ds.enumerate() { cell(0, 3.6 - i * 0.5, 4.0, 0.42, d) }
  arrow((4.3, 2.1), (5.4, 2.1))
  label(5.7, 4.3, [AEV index: sorted by (A, E, V), one key per datom], size: 7.6pt, fill: ink)
  let aev = ("[:g/to 1 2 tx1]", "[:g/to 1 3 tx1]", "[:g/to 2 3 tx1]", "[:g/to 2 4 tx1]", "[:g/to 3 4 tx1]", "[:g/to 4 1 tx2]", "[:g/to 5 1 tx1]")
  for (i, d) in aev.enumerate() { cell(5.7, 3.6 - i * 0.5, 3.6, 0.42, d, fill: accent-soft, stroke: accent) }
  for (e, r0, r1) in ((1, 0, 1), (2, 2, 3), (3, 4, 4), (4, 5, 5), (5, 6, 6)) {
    let ytop = 3.6 - r0 * 0.5 + 0.42; let ybot = 3.6 - r1 * 0.5
    draw.line((9.45, ytop), (9.55, ytop), (9.55, ybot), (9.45, ybot), stroke: 0.6pt + accent)
    label(9.7, (ytop + ybot) / 2, [entity ] + str(e) + [: ] + str(r1 - r0 + 1) + (if r1 == r0 { [ key] } else { [ adjacent keys] }), size: 7pt)
  }
  label(0, -0.35, [AVE holds the same seven datoms sorted by (A, V, E). Its first two keys are \[:g/to 1 4\] and \[:g/to 1 5\]: the two entities that point to 1, again adjacent.], size: 7pt)
}), [Datoms become sorted keys. One entity's edges form a contiguous key range in AEV. A read of them is one seek and a short range walk. Every optimization in this report changes either the shape of these keys or the way the join reads them.])

== How Triplox runs the triangle query

A triangle is three vertices joined by three edges in one direction: a→b, b→c, and the closing edge a→c. The query below asks for every one. It is the standard test of a graph join. The third pattern refers back to the first variable, so the join cannot run as a simple chain. The example graph holds two.

#fig(canvas(length: 1cm, {
  draw.content((0, 3.3), anchor: "west", ctext("[:find ?a ?b ?c :where [?a :g/to ?b] [?b :g/to ?c] [?a :g/to ?c]]", size: 7.6pt))
  example-graph(1.6, 0.0, hl: (("a", "b"), ("b", "c"), ("a", "c")))
  draw.content((1.6, -0.35), stext([?a=1, ?b=2, ?c=3], size: 7pt, fill: rust))
  example-graph(7.6, 0.0, hl: (("b", "c"), ("c", "d"), ("b", "d")))
  draw.content((7.6, -0.35), stext([?a=2, ?b=3, ?c=4], size: 7pt, fill: rust))
  draw.content((11.2, 1.2), anchor: "west", stext([result: two rows], size: 7pt, fill: ink))
  draw.content((11.2, 0.85), anchor: "west", ctext("(1 2 3)", size: 7pt))
  draw.content((11.2, 0.55), anchor: "west", ctext("(2 3 4)", size: 7pt))
}), [The triangle query on the example graph. Each highlighted set of three edges satisfies all three patterns at once. The edge d→a does not close a triangle because no vertex points to both d and a.])

The query compiler orders the variables ?a, ?b, ?c. The #term[generic join] then extends bindings one variable at a time. At each level it asks every pattern that mentions the variable for candidate values, intersects the candidate sets, and recurses into each survivor. This is a worst-case-optimal join. On the triangle, the third pattern `[?a :g/to ?c]` prunes ?c at once, and the join never materializes the full a→b→c path list. Every binding at every level performs its own index seek.

#fig(canvas(length: 1cm, {
  label(0, 4.6, [?a  ← scan AEV :g/to, distinct entities], size: 7.4pt, fill: ink)
  strip(0, 3.9, 0.7, 0.42, ("1", "2", "3", "4", "5"))
  label(3.8, 4.1, [5 bindings], size: 7pt)
  label(5.2, 3.3, [?b  ← AEV\[a\]: one seek per binding of ?a], size: 7.4pt, fill: ink)
  strip(0, 2.6, 0.7, 0.42, ("2", "3", "3", "4", "4", "1", "1"))
  label(5.2, 2.8, [7 bindings, one per edge], size: 7pt)
  for (i, t) in ("a=1", "a=1", "a=2", "a=2", "a=3", "a=4", "a=5").enumerate() { label(0.35 + i * 0.7, 2.4, t, size: 6.3pt, anchor: "center") }
  for (i, p) in (0, 0, 1, 1, 2, 3, 4).enumerate() { draw.line((0.35 + p * 0.7, 3.9), (0.35 + i * 0.7, 3.02), stroke: 0.5pt + accent-mid) }
  for (i, ok) in (true, false, true, false, false, false, false).enumerate() { if ok { draw.line((0.35 + i * 0.7, 2.6), (1.25 + (if i == 0 { 0 } else { 2.6 }), 1.42), stroke: 0.5pt + accent-mid) } }
  label(4.3, 1.75, [?c  ← AEV\[b\] ∩ AEV\[a\]: two seeks per binding of (a, b), then intersect], size: 7.4pt, fill: ink)
  let rs = ("{3,4}∩{2,3}={3}", "{4}∩{2,3}=∅", "{4}∩{3,4}={4}", "{1}∩{3,4}=∅", "{1}∩{4}=∅", "{2,3}∩{1}=∅", "{2,3}∩{1}=∅")
  for (i, r) in rs.enumerate() {
    let ok = i == 0 or i == 2
    let col = calc.rem(i, 4); let row = calc.floor(i / 4)
    cell(col * 2.6, 1.0 - row * 0.52, 2.5, 0.42, r, fill: if ok { accent-soft } else { white }, stroke: if ok { accent } else { rule })
  }
  label(0, -0.25, [results (1,2,3) and (2,3,4): 14 index seeks and 7 intersections to find 2 triangles], size: 7.4pt, fill: ink)
}), [The generic join on the example. Each box at a level is one binding. Each binding costs at least one seek into AEV at the next level. On the benchmark graph the ?b level has 39,764 bindings, and each one performs two seeks. Seek count and per-binding bookkeeping dominate the run time.])

== Queries

Each query isolates one access path. The row count is the output. Every experiment and configuration compared it against the baseline.

#table(columns: (auto, 1fr, auto, auto),
  table.header([name], [Datalog and what it exercises], align(right)[rows], align(right)[ms]), hlh,
  [triangles], [#mono[\[:find ?a ?b ?c :where \[?a :g/to ?b\] \[?b :g/to ?c\] \[?a :g/to ?c\]\]] \ cyclic three-way join, the case worst-case-optimal joins are designed for], align(right)[7,821], align(right)[537], hl,
  [two_hop_count], [#mono[\[:find (count ?c) :where \[?a :g/to ?b\] \[?b :g/to ?c\]\]] \ fan-out over two reference hops, then an aggregate], align(right)[1], align(right)[223], hl,
  [three_hop_count], [#mono[\[:find (count ?d) :where \[?a :g/to ?b\] \[?b :g/to ?c\] \[?c :g/to ?d\]\]] \ three hops, about 16 million intermediate rows before the count], align(right)[1], align(right)[7,041], hl,
  [out_degree], [#mono[\[:find ?a (count ?b) :where \[?a :g/to ?b\]\]] \ full AEV scan of one attribute, grouped by entity], align(right)[2,000], align(right)[44], hl,
  [in_degree_top], [#mono[\[:find ?b (count ?a) :where \[?a :g/to ?b\]\]] \ the same scan, grouped by value], align(right)[2,000], align(right)[44], hl,
  [neighbors_of_42], [#mono[\[:find ?b :where \[?a :g/id 42\] \[?a :g/to ?b\]\]] \ unique-attribute lookup, then one entity's edges], align(right)[21], align(right)[1.1], hl,
  [weight_filter], [#mono[\[:find ?e :where \[?e :g/weight ?w\] \[(> ?w 900)\]\]] \ scalar scan with a range predicate], align(right)[198], align(right)[3.7], hl,
  [weight_sum], [#mono[\[:find (sum ?w) :where \[?e :g/weight ?w\]\]] \ single-column aggregate, the nearest thing to an analytical scan], align(right)[1], align(right)[3.6], hl,
  [heavy_neighbors], [#mono[\[:find ?a ?b :where \[?a :g/to ?b\] \[?b :g/weight ?w\] \[(> ?w 950)\]\]] \ a reference hop joined with a scalar predicate], align(right)[1,908], align(right)[68], hl,
  [label_lookup], [#mono[\[:find ?e :where \[?e :g/label "label-777"\]\]] \ string equality through AVE, a control for fixed overhead], align(right)[1], align(right)[0.02], hl)

== Results overview

One cell per experiment and query, as a speed-up factor. Each chapter shows its own row as a chart of absolute times with every sample, and the appendix holds the medians, intervals and p-values.

#fig(heatmap(), [Median over median. The chapter 7 rows compare each stack's all-on arm with that stack's own off arm.])

#fig(slopegraphs(), [Each line runs from a query's time with everything off to its time with the technique on. Grey: the 95% interval of the ratio includes 1.])

// ==================================================================
#pagebreak()
= Vectorized join #toggle[exp/vectorized-join · TRIPLOX_BATCHED_JOIN=1]

== Where the time goes today

The generic join in chapter 1 keeps its intermediate state as rows. Each binding is a `Vec<Bytes>` with one `Bytes` per bound variable. To extend a level, the join allocates a new row for every parent row, clones the parent's values into it, and appends the new value. The join builds candidate sets the same way, one small vector per candidate, grouped through a `BTreeMap`. None of this is the query's work. It is bookkeeping around the work.

A sampling profile of the triangle query on the benchmark graph shows the effect. The shares below are shares of total query time.

#fig(canvas(length: 1cm, {
  let parts = (("malloc", 34, rust, white), ("memcpy / memcmp", 20, rgb("#c9776a"), white), ("Bytes::cmp, refcounts", 9, rgb("#dcaea6"), ink), ("SlateDB seek + next", 28, accent, white), ("other", 9, rule, ink))
  let x = 0
  let W = 14.0
  for (n, p, c, tc) in parts {
    let w = W * p / 100
    draw.rect((x, 0), (x + w, 0.55), fill: c, stroke: 0.5pt + white)
    draw.content((x + w / 2, 0.27), text(font: "Source Sans 3", size: 7pt, fill: tc, str(p) + "%"))
    draw.content((x + w / 2, -0.3), stext(n, size: 6.8pt))
    x = x + w
  }
  draw.content((W * 0.315, 1.0), stext([row bookkeeping: 63%], size: 7.4pt, fill: rust))
  draw.content((W * 0.77, 1.0), stext([index iteration: 28%], size: 7.4pt, fill: accent))
}), [Where the triangle query spends its time before the change, from a 20 second sample of the release binary. Allocation, copying and byte comparison of row vectors take twice as long as reading the index.])

== The idea: process columns, not rows

DuckDB follows the MonetDB/X100 design and never hands one row from operator to operator. Each operator takes a #term[vector] of approximately two thousand values for one column. It does its work in a tight loop over that array and passes the array on. Allocation happens once per vector instead of once per value. The loop body is small, the same instruction runs over adjacent memory, and the compiler keeps the loop in registers.

The second half of the idea is the layout of a set of rows when the query has several variables. Instead of one array per row, keep one array per variable, all of the same length. Row #emph[i] is then the #emph[i]-th entry of each array. This layout is called #term[structure of arrays]. Projection drops an array. An aggregate over one variable reads one contiguous array.

== Mapping onto the generic join

The generic join grows its bindings one variable at a time. Each level of the join is a natural vector: the column of new values for that variable. Each new value belongs to exactly one parent binding at the level above. The new level stores the #term[index] of its parent instead of a copy of the parent's values. The engine reconstructs a row only when it needs one, by following parent indexes upward.

#fig(canvas(length: 1cm, {
  label(0, 4.3, [row engine: every binding is its own Vec<Bytes>], size: 7.6pt, fill: ink)
  label(0, 3.9, [level ?a], size: 7pt); strip(1.4, 3.7, 0.55, 0.4, ("1", "2", "3", "4", "5"), size: 7pt)
  label(0, 3.35, [level ?b], size: 7pt)
  let l2 = ("1,2", "1,3", "2,3", "2,4", "3,4", "4,1", "5,1")
  for (i, r) in l2.enumerate() { cell(1.4 + i * 0.8, 3.15, 0.75, 0.4, r, size: 6.8pt) }
  label(0, 2.8, [level ?c], size: 7pt)
  cell(1.4, 2.6, 1.0, 0.4, "1,2,3", size: 6.8pt); cell(2.5, 2.6, 1.0, 0.4, "2,3,4", size: 6.8pt)
  label(0, 2.2, [14 Vec allocations and 11 Bytes clones, plus one Vec per candidate value at every level], size: 7pt, fill: rust)

  label(0, 1.9, [batched engine: one column per level, one parent index per entry], size: 7.6pt, fill: ink)
  label(0, 1.35, [?a values], size: 7pt); strip(1.4, 1.15, 0.55, 0.4, ("1", "2", "3", "4", "5"), fills: (accent-soft,) * 5, stroke: accent, size: 7pt)
  label(0, 0.55, [?b parent], size: 7pt); strip(1.4, 0.35, 0.55, 0.4, ("0", "0", "1", "1", "2", "3", "4"), size: 7pt)
  label(0, 0.15, [?b values], size: 7pt); strip(1.4, -0.05, 0.55, 0.4, ("2", "3", "3", "4", "4", "1", "1"), fills: (accent-soft,) * 7, stroke: accent, size: 7pt)
  for (i, p) in (0, 0, 1, 1, 2, 3, 4).enumerate() {
    draw.line((1.4 + i * 0.55 + 0.275, 0.75), (1.4 + p * 0.55 + 0.275, 1.15), stroke: 0.55pt + accent-mid, mark: (end: "straight", scale: 0.4, fill: accent-mid))
  }
  label(5.6, 0.35, [entry 2 of ?b has parent 1: value 3 extends ?a\[1\] = 2], size: 6.8pt)
  label(0, -0.55, [?c parent], size: 7pt); strip(1.4, -0.75, 0.55, 0.4, ("0", "2"), size: 7pt)
  label(0, -0.95, [?c values], size: 7pt); strip(1.4, -1.15, 0.55, 0.4, ("3", "4"), fills: (accent-soft,) * 2, stroke: accent, size: 7pt)
  for (i, p) in (0, 2).enumerate() {
    draw.line((1.4 + i * 0.55 + 0.275, -0.35), (1.4 + p * 0.55 + 0.275, -0.05), stroke: 0.55pt + accent-mid, mark: (end: "straight", scale: 0.4, fill: accent-mid))
  }
  label(5.6, -0.75, [each arrow is a parent index: it points at the entry one level up], size: 6.8pt)
  label(0, -1.65, [3 value columns and 2 parent columns, each allocated once. Row (2,3,4) is read as ?c\[1\] → parent 2 → ?b\[2\]=3 → parent 1 → ?a\[1\]=2.], size: 7pt, fill: accent)
}), [The three levels of the triangle join on the example, in both representations. In the row engine the number of allocations equals the number of bindings. In the batched engine it equals the number of levels.])

The implementation adds three parts, and the code and the toggle call the result the batched join. A `Batch` type holds these columns with structural sharing. A `BatchedJoinEngine` drives the existing patterns level by level over whole batches. A fast path runs comparison predicates over a column. Grouping for aggregates sorts an array of row indexes instead of building a `BTreeMap`. Queries with `not`, `or`, relation or function clauses go to the existing engine unchanged. The batched engine only sees pure triple patterns, predicates and aggregates.

== Walkthrough: three_hop_count

The query `[:find (count ?d) :where [?a :g/to ?b] [?b :g/to ?c] [?c :g/to ?d]]` is a chain. No pattern closes a cycle, and no candidate is pruned. The ?d level holds approximately sixteen million entries. The row engine builds sixteen million four-element vectors, and each one carries three cloned `Bytes`. The batched engine builds one column of sixteen million values and one column of sixteen million parent indexes. The count reads the length of one array. The deeper the join, the larger the share of time that was row bookkeeping, and the larger the gain in the chart below.

== Results

#stats-chart("vectorized-join")

#verdict(
  [Correctness], [Results are identical to the current engine including row order. 642 tests pass with the toggle on and off. Clippy is clean.],
  [Reading the chart], [The gain grows with join depth. One level is flat, two levels give approximately 0.6×, and three levels give 0.28×. Single-scan queries are unaffected. The small differences on them are noise.],
  [What is not done], [The final level is still fully materialized before an aggregate runs. A columnar aggregate sink would let `three_hop_count` count without building the ?d column. The bounded per-level batching from issue 204 is also not implemented. A level is columnar but not chunked, and peak memory is unchanged.],
  [Cost to integrate], [Low. No storage format, change feed or planner changes. The risk is the duplicated pattern-validation logic in `triple.rs`. Only the equivalence test holds the two copies together.])

// ==================================================================
#pagebreak()
= Multiple datoms per key #toggle[exp/datom-segments · TRIPLOX_SEGMENT_SIZE=256]

== Where the time goes today

SlateDB is a log-structured merge tree. Keys live in sorted string tables. Each table is split into blocks and has a block index and a Bloom filter. A read of one key finds the table, binary-searches the block index, loads the block, and scans to the key. A range iteration repeats the per-key part for every key: compare the key against the range end, decode the entry, copy it into a `Bytes`, and advance.

With one datom per key, the `:g/to` attribute is 39,764 keys in AEV. A full scan for `out_degree` performs 39,764 of those steps. Every seek in the triangle join lands on a single 27-byte key.

== The idea: segments

Datomic does not store one datom per storage key. Its indexes are trees. Each leaf is a #term[segment] of thousands of datoms, stored as one value under one key. The storage layer sees a few thousand large values instead of millions of small ones. A scan pays the per-key cost once per segment and then walks an in-memory array. A point lookup fetches one segment and binary-searches inside it.

The price is write amplification. To add one datom, the writer rewrites the segment that owns it. A segment that grows past its limit splits in two. Datomic accepts this because it batches writes by transaction and rebuilds its indexes in the background.

== Mapping onto SlateDB

This experiment keeps the per-datom encoding as it is and changes only how many datoms share a key. Up to N sorted datom keys are concatenated into one value, each with a length prefix. The SlateDB key of a segment is the key of the #term[last] datom in it.

SlateDB's API forces that choice. An iterator can seek forward to the first key at or after a target, but it cannot step backward. Suppose the key were the first datom. A seek to a datom in the middle of a segment then lands on the #emph[next] segment and misses it. With segments keyed by their last datom, the seek lands on the segment that owns the target.

#fig(canvas(length: 1cm, {
  label(0, 4.1, [today: seven keys, each with an empty value], size: 7.6pt, fill: ink)
  let aev = ("[:g/to 1 2]", "[:g/to 1 3]", "[:g/to 2 3]", "[:g/to 2 4]", "[:g/to 3 4]", "[:g/to 4 1]", "[:g/to 5 1]")
  for (i, k) in aev.enumerate() { cell(i * 2.0, 3.4, 1.95, 0.42, k, size: 6.8pt) }
  label(0, 3.05, [scan of :g/to: 7 key steps · seek to entity 3: binary search, then 1 key step], size: 7pt)

  label(0, 2.45, [segments of size 4: two keys, each holding several datoms], size: 7.6pt, fill: ink)
  cell(0, 1.5, 4.8, 0.42, "key [:g/to 2 4]", fill: accent-soft, stroke: accent, size: 6.8pt)
  cell(0, 1.05, 4.8, 0.42, "value: [1 2] [1 3] [2 3] [2 4]", fill: white, stroke: accent, size: 6.4pt)
  cell(5.1, 1.5, 4.8, 0.42, "key [:g/to 5 1]", fill: accent-soft, stroke: accent, size: 6.8pt)
  cell(5.1, 1.05, 4.8, 0.42, "value: [3 4] [4 1] [5 1]", fill: white, stroke: accent, size: 6.4pt)
  label(10.1, 1.27, [← key = last datom], size: 6.8pt)
  draw.content((7.5, 2.75), stext([seek \[:g/to 3\]], size: 6.8pt, fill: rust))
  draw.line((7.5, 2.6), (7.5, 1.97), stroke: 0.9pt + rust, mark: (end: "straight", scale: 0.55, fill: rust))
  draw.line((4.85, 1.71), (5.05, 1.71), stroke: (paint: rust, thickness: 0.9pt), mark: (end: "straight", scale: 0.5, fill: rust))
  label(0, 0.25, [seek \[:g/to 3\]: the first key \[:g/to 2 4\] is smaller than the target, so the forward seek lands on \[:g/to 5 1\], the segment that owns entity 3], size: 6.8pt, fill: rust)
  label(0, 0.6, [scan of :g/to: 2 key steps, then in-memory iteration], size: 7pt)
}), [Segments on the example with N = 4. The benchmark uses N = 256, and the AEV keys for `:g/to` drop from 39,764 to 156. A forward seek to any datom returns the segment whose last datom is the first one at or after the target. That segment owns the target.])

Reads go through a `KeyCursor` that accepts both layouts in one index. The writer's segment size can change between transactions. Writes merge new datoms into the owning segment. At flush time, a segment that exceeds N splits once.

== Interaction with the incremental engine

The incremental engine does not use the ad-hoc query path. It seeds a DBSP circuit with one EAV scan and then feeds it from the SlateDB change feed, one entry at a time. Today one entry is one datom. With segments, one entry is a whole segment, and a rewrite republishes every datom in it.

#fig(canvas(length: 1cm, {
  let seg(x, y, items, hl: (), title: none) = {
    if title != none { draw.content((x, y + 0.65), anchor: "west", stext(title, size: 6.8pt, fill: ink2)) }
    for (i, it) in items.enumerate() {
      let h = i in hl
      cell(x + i * 1.75, y, 1.7, 0.42, it, fill: if h { accent-soft } else { white }, stroke: if h { accent } else { rule }, size: 6.6pt)
    }
  }
  draw.content((0, 4.5), anchor: "west", stext([transaction 2 asserts one datom, \[4 :g/to 1\]], size: 7.6pt, fill: ink, weight: 600))
  seg(0, 3.4, ("[3 4 tx1]", "[4 1 tx2]", "[5 1 tx1]"), hl: (1,), title: [the segment that owns it is rewritten with the new datom merged in])
  arrow((5.4, 3.61), (6.6, 3.61))
  draw.content((6.75, 3.61), anchor: "west", stext([one change-feed entry: 3 datoms, 1 of them new], size: 7pt, fill: ink))

  draw.content((0, 2.5), anchor: "west", stext([subscriber that counts :g/to edges, before the fix], size: 7.6pt, fill: rust, weight: 600))
  seg(0, 1.55, ("[3 4 tx1]", "[4 1 tx2]", "[5 1 tx1]"), hl: (0, 1, 2), title: [every datom in the entry is treated as new])
  arrow((5.4, 1.76), (6.6, 1.76), stroke: rust)
  draw.content((6.75, 1.76), anchor: "west", stext([count 6 → 9. The true answer is 7.], size: 7pt, fill: rust))

  draw.content((0, 0.7), anchor: "west", stext([after the fix in the CDC reader], size: 7.6pt, fill: accent, weight: 600))
  seg(0, -0.25, ("[3 4 tx1]", "[4 1 tx2]", "[5 1 tx1]"), hl: (1,), title: [keep only datoms whose tx equals the newest tx in the entry])
  arrow((5.4, -0.04), (6.6, -0.04), stroke: accent)
  draw.content((6.75, -0.04), anchor: "west", stext([count 6 → 7], size: 7pt, fill: accent))
}), [What the change feed carries once several datoms share a key. A segment rewrite republishes the datoms that transaction 1 wrote, and a subscriber that trusts the entry counts them again. The fix filters each datom by its own transaction id. The seeding scan needed the same change. The filter assumes an entry never carries more than one new transaction, which the current write path guarantees but nothing enforces.])

The adjacency matrices of chapter 5 and the zone maps of chapter 6 are caches keyed by basis. Neither is updated from the change feed, so each new transaction forces a rebuild on the next query. The vectorized join changes only the ad-hoc engine and has no effect on incremental queries.

== Walkthrough: out_degree and neighbors_of_42

`out_degree` scans all of `:g/to` in AEV. Today the scan takes 39,764 iterator steps. With N = 256 it takes 156 steps. Each step returns one value, which the reader decodes into an array and walks in memory. `neighbors_of_42` seeks to one entity. Today that is a binary search and about twenty key steps. After the change it is a binary search and one segment decode. These two queries show the largest improvement in the chart.

The triangle join seeks once per binding, 39,764 times at the ?b level. Each seek now decodes a 256-datom segment to read about twenty datoms from it. The per-seek cost rises while the per-key cost falls. The join-heavy queries therefore improve less than the scans.

== Results

#stats-chart("datom-segments")

#verdict(
  [Correctness], [643 tests pass at size 1 and at size 256. An equivalence test compares 11 queries, current and as-of, at sizes 1, 3, 16 and 256. That test found two silent write-path defects, both of which dropped or duplicated datoms during a split. Both are fixed.],
  [Storage], [The EAV key count fell from 45,953 to 224. Total bytes rose 8% from the length prefix per datom. Gains flatten past size 256.],
  [Reading the chart], [The scans and the point lookup gain the most. The joins gain less, and `three_hop_count` sits at the edge of no effect. Its interval touches 1, and the report counts it as unchanged.],
  [Cost to integrate], [A segment rewrite republishes the datoms of earlier transactions through the SlateDB change feed. This broke all 38 incremental-query tests until the CDC layer learned to keep only the newest transaction's datoms. Write amplification under random single-datom writes was not measured. Segment values must stay under SlateDB's key and value limits.])

// ==================================================================
#pagebreak()
= Columnar segments #toggle[exp/columnar-storage · TRIPLOX_SEGMENT_LAYOUT=columnar]

== Where the time goes today

Chapter 3 reduces the number of keys a scan touches. Inside a segment, the datoms still sit one after another with all their components. A query that needs only the entity of each datom still reads and decodes its value, transaction id and flag. Nothing is compressed. An entity id takes nine bytes wherever it appears, even in a segment that holds two hundred consecutive datoms for the same entity.

== The idea: store each component as its own array, then compress it

A #term[columnar] layout stores a segment as one array per component instead of one record per datom. The same seven datoms become an array of entities, an array of values, a bitmap of flags and an array of transaction ids. A reader that needs one component reads one array and skips the rest. Analytical databases such as DuckDB use this layout throughout for that reason. Each array holds values of one kind, and values of one kind compress well.

The compression is the standard pair from DuckDB and Parquet. #term[Frame of reference] subtracts the array's minimum from every value. A column of large integers becomes a column of small offsets. #term[Bit packing] then stores each offset in the smallest number of bits that fits the largest offset, instead of 64. A column of 256 sorted entity ids that span a range of 1000 needs 10 bits per entry, 320 bytes in total. The same column at 9 bytes per entry takes 2304 bytes.

#fig(canvas(length: 1cm, {
  label(0, 5.6, [row-major segment: 7 records × (9 + 9 + 1 + 8 bytes) = 189 bytes], size: 7.6pt, fill: ink)
  let recs = (("E=1", "V=2", "+", "tx1"), ("E=1", "V=3", "+", "tx1"), ("E=2", "V=3", "+", "tx1"), ("E=2", "V=4", "+", "tx1"), ("E=3", "V=4", "+", "tx1"), ("E=4", "V=1", "+", "tx2"), ("E=5", "V=1", "+", "tx1"))
  for (i, r) in recs.enumerate() {
    let x = i * 2.02
    cell(x, 4.9, 0.55, 0.4, r.at(0), size: 6.4pt); cell(x + 0.55, 4.9, 0.55, 0.4, r.at(1), size: 6.4pt)
    cell(x + 1.1, 4.9, 0.3, 0.4, r.at(2), size: 6.4pt); cell(x + 1.4, 4.9, 0.58, 0.4, r.at(3), size: 6.4pt)
  }
  label(0, 4.5, [a reader wanting only entities must step through every record], size: 7pt, fill: rust)

  label(0, 3.7, [columnar segment: one array per component, integers as frame of reference + bit packing], size: 7.6pt, fill: ink)
  label(0, 3.2, [E], size: 7.4pt, fill: ink); strip(0.8, 3.0, 0.55, 0.4, ("1", "1", "2", "2", "3", "4", "5"), fills: (accent-soft,) * 7, stroke: accent, size: 6.8pt)
  label(4.9, 3.2, [min 1 → offsets 0 0 1 1 2 3 4 → max 4 fits in 3 bits → 21 bits ≈ 3 bytes], size: 6.8pt)
  label(0, 2.6, [V], size: 7.4pt, fill: ink); strip(0.8, 2.4, 0.55, 0.4, ("2", "3", "3", "4", "4", "1", "1"), fills: (accent-soft,) * 7, stroke: accent, size: 6.8pt)
  label(4.9, 2.6, [min 1 → offsets 1 2 2 3 3 0 0 → max 3 fits in 2 bits → 14 bits ≈ 2 bytes], size: 6.8pt)
  label(0, 2.0, [op], size: 7.4pt, fill: ink); strip(0.8, 1.8, 0.55, 0.4, ("+", "+", "+", "+", "+", "+", "+"), size: 6.8pt)
  label(4.9, 2.0, [bitmap, 1 bit each → 1 byte], size: 6.8pt)
  label(0, 1.4, [tx], size: 7.4pt, fill: ink); strip(0.8, 1.2, 0.55, 0.4, ("1", "1", "1", "1", "1", "2", "1"), size: 6.8pt)
  label(4.9, 1.4, [min 1 → offsets 0 0 0 0 0 1 0 → 1 bit → 1 byte], size: 6.8pt)
  label(0, 0.7, [about 7 bytes of data plus a small header per column; out_degree reads the E array only], size: 7pt, fill: accent)
  label(0, 0.3, [the E array bit-packed at 3 bits per entry:], size: 6.8pt)
  strip(4.6, 0.1, 0.55, 0.36, ("000", "000", "001", "001", "010", "011", "100"), fills: (accent-soft,) * 7, stroke: accent, size: 6.2pt)
}), [The seven `:g/to` datoms as a row-major segment and as a columnar segment. Every column carries its own byte length, and a reader jumps over the columns it does not need. The benchmark's segments hold up to 1024 datoms. At that size the ratio is far larger than on seven.])

== Mapping onto the indexes

The layout applies to AEV and AVE, the two indexes graph queries use. The AE and AV indexes exist to enumerate distinct entities or distinct values for an attribute. In the new layout they are the first column of an AEV or AVE segment with duplicates collapsed. The experiment removes them and serves those scans from the first column, which is part of the storage reduction below. Temporal filtering reads the transaction column and resolves the visible version of each logical key from it. EAV and VAE keep the row layout.

== Walkthrough: out_degree and three_hop_count

`out_degree` groups entities in AEV. The columnar scan reads the E column of each of the 55 segments for `:g/to`. It decodes 3 bytes per entry and never touches values or transactions. That gives the 0.30× in the chart.

`three_hop_count` performs sixteen million single-row seeks. Each seek decodes a whole segment's E and V columns to find one row. The decode cancels the key-count saving. In the first A/B, run while other experiments were benchmarking, the query regressed to 1.25×. On the idle machine the ratio is 0.97 with an interval that includes 1, so the deep join neither gains nor loses. The row-major segments of chapter 3 show the same shape. Chapter 7 tests whether batched seeks turn this into a gain.

== Results

#stats-chart("columnar-storage")

#verdict(
  [Correctness], [645 tests pass with the toggle off. With the columnar layout forced on for the whole suite, 10 library tests fail. Each is a test-harness artifact, such as a test that writes raw row keys directly. The branch's EXPERIMENT.md lists them. The equivalence test exposed an off-by-one in the first-column decoder that made every AE/AV scan panic. It is fixed.],
  [Storage], [At scale, bytes fell 61% and key count 69%. AEV for `:g/to` went from 46,173 keys to 55.],
  [Point lookups], [`label_lookup` is a 20 µs query, and its ratio is noise in every single experiment. Inside the full stack the columnar arm slows it from 0.02 to 0.03 ms with an interval that excludes 1: a string lookup on AVE now decodes a segment to find one key. The per-segment index proposed above would remove that cost.],
  [Cost to integrate], [The layout is a property of a database. It must be recorded in bootstrap metadata, not read from an environment variable. Compaction and updates rewrite whole segments. The change feed issue from chapter 3 applies unchanged.])

// ==================================================================
#pagebreak()
= Sparse adjacency matrices #toggle[exp/sparse-matrix · TRIPLOX_ADJ_MATRIX=1]

== Where the time goes today

Every graph query in the benchmark is a sequence of "follow the edges of #emph[x]" steps. Each step is an index seek: find the key range for entity #emph[x] in AEV, iterate it, decode each datom, and hand the values back as `Bytes`. The join in chapter 1 performs that seek once per binding. The triangle query on the benchmark graph performs about 80,000 seeks to find 7,821 triangles. Every seek returns the same twenty or so neighbours that an earlier seek already read.

== The idea: the graph as a matrix

A directed graph on #emph[n] vertices can be written as an #emph[n] × #emph[n] #term[adjacency matrix] #emph[A]. Entry (#emph[i], #emph[j]) is 1 if there is an edge from #emph[i] to #emph[j] and 0 otherwise. Row #emph[i] lists the out-neighbours of #emph[i]. Column #emph[j] lists the in-neighbours of #emph[j]. The product of #emph[A] with itself gives, at (#emph[i], #emph[k]), the number of two-step paths from #emph[i] to #emph[k]. The triangles are the entries of #emph[A]² that also have a direct edge in #emph[A].

Almost all entries of a real adjacency matrix are 0. The benchmark graph has 4 million cells and 39,764 ones. A #term[sparse] representation stores only the ones.

The standard form is #term[compressed sparse row], CSR. One array holds the column indices of all the ones, row after row. A second array of offsets says where each row starts. Row #emph[i] is the slice between offsets[#emph[i]] and offsets[#emph[i]+1]. RedisGraph stored every relationship type as such a matrix with the GraphBLAS library and compiled Cypher pattern matching to sparse matrix algebra.

#fig(canvas(length: 1cm, {
  label(0, 5.4, [adjacency matrix A of the example], size: 7.6pt, fill: ink)
  let ones = ((1, 2), (1, 3), (2, 3), (2, 4), (3, 4), (4, 1), (5, 1))
  for j in range(1, 6) { draw.content((0.9 + (j - 1) * 0.5, 4.95), ctext(str(j), size: 6.8pt)) }
  for i in range(1, 6) {
    draw.content((0.35, 4.55 - (i - 1) * 0.5), ctext(str(i), size: 6.8pt))
    for j in range(1, 6) {
      let one = (i, j) in ones
      draw.rect((0.65 + (j - 1) * 0.5, 4.3 - (i - 1) * 0.5), (1.15 + (j - 1) * 0.5, 4.8 - (i - 1) * 0.5), fill: if one { accent-soft } else { white }, stroke: 0.4pt + rule)
      draw.content((0.9 + (j - 1) * 0.5, 4.55 - (i - 1) * 0.5), ctext(if one { "1" } else { "·" }, size: 6.8pt))
    }
  }
  label(0, 1.6, [rows: from, columns: to], size: 6.8pt)
  label(0, 1.3, [25 cells, 7 ones. Benchmark:], size: 6.8pt); label(0, 1.0, [4 M cells, 39,764 ones], size: 6.8pt)

  label(4.2, 5.4, [compressed sparse row: only the ones, row by row], size: 7.6pt, fill: ink)
  for (i, t) in ("0", "1", "2", "3", "4", "5", "6").enumerate() { draw.content((5.775 + i * 0.55, 4.95), ctext(t, size: 6pt)) }
  label(4.2, 4.6, [targets], size: 7.2pt, fill: ink); strip(5.5, 4.4, 0.55, 0.4, ("2", "3", "3", "4", "4", "1", "1"), fills: (accent-soft,) * 7, stroke: accent, size: 6.8pt)
  for (i, (s, e)) in ((0, 2), (2, 4), (4, 5), (5, 6), (6, 7)).enumerate() {
    draw.line((5.5 + s * 0.55 + 0.05, 4.32), (5.5 + e * 0.55 - 0.05, 4.32), stroke: 1.2pt + (if calc.rem(i, 2) == 0 { accent } else { accent-mid }))
  }
  label(4.2, 3.7, [offsets], size: 7.2pt, fill: ink); strip(5.5, 3.5, 0.55, 0.4, ("0", "2", "4", "5", "6", "7"), size: 6.8pt)
  for (i, t) in ("row1", "row2", "row3", "row4", "row5", "end").enumerate() { draw.content((5.775 + i * 0.55, 3.3), ctext(t, size: 5.6pt)) }
  label(4.2, 2.6, [row(1) = targets\[offsets\[1\] .. offsets\[2\]\] = targets\[0..2\] = {2, 3}], size: 7pt)
  label(4.2, 2.2, [out-degree(1) = offsets\[2\] − offsets\[1\] = 2], size: 7pt)
  label(4.2, 1.6, [triangle through edge 1→2: row(1) ∩ row(2) = {2,3} ∩ {3,4} = {3}], size: 7pt, fill: accent)
  label(4.2, 1.2, [both rows are sorted, so the intersection is one merge pass, no seeks], size: 7pt)
  label(4.2, 0.6, [a second CSR built from AVE gives column slices: in-neighbours of 1 = {4, 5}], size: 7pt)
}), [The example graph as a dense matrix and as CSR. Row lookups, degrees and intersections become slices of two integer arrays. The benchmark's `:g/to` matrix takes 1.45 MB for both orientations, 36 bytes per edge, and about 40 ms to build.])

== Mapping onto Triplox

A reference attribute in Triplox is a relationship type. The AEV index for it, read in order, is the CSR targets array: entity after entity, each followed by its sorted values. One AEV scan builds the matrix and records where each entity's run starts. The AVE index gives the transposed matrix the same way. Entity ids are integers. The row index of an entity is its position in a dense id map built during the same scan.

The matrix is built per reference attribute and per database basis, the transaction id a query reads at. The node caches it keyed by (attribute, basis). The matrix is a derived structure. The index remains the source of truth, and an as-of query at a different basis uses the existing path.

In the planner, an `AdjacencyPattern` replaces any triple pattern whose attribute is a reference type. It satisfies the same `ExecPattern` contract as a triple pattern but answers from the matrix. A bound entity gives a row slice, and a bound value gives a column slice. Both bound give a binary search, and neither bound gives the full edge list. Scalar patterns are untouched. `[?b :g/weight ?w]` in `heavy_neighbors` still goes to the index.

== Walkthrough: triangles

For each edge (a, b) in the full edge list, the third variable is row(a) ∩ row(b). Both rows are sorted slices of the targets array. The intersection is one merge pass over approximately forty integers. The whole query is about 40,000 merge passes over memory-resident arrays, with no allocation per binding. That gives the 0.03× in the chart. `out_degree` reads the offsets array once. `neighbors_of_42` is one slice.

== Results

#stats-chart("sparse-matrix")

#verdict(
  [Correctness], [642 tests pass on and off. The equivalence test covers 11 queries including a retraction.],
  [Reading the chart], [Queries on the reference attribute improve by 1.1× to 45×. The three scalar queries are unchanged, as they must be. `heavy_neighbors` improves less than the pure traversals because its scalar predicate still seeks the index once per candidate.],
  [Side finding], [In the first A/B, `heavy_neighbors` was 9.6× slower with the matrix on. The cause was a pre-existing planner defect, described in chapter 8. The matrix reported an exact candidate count and lost the proposer choice to an index estimate of zero. Two changes to candidate-set handling in the join fixed it without a change to the estimator.],
  [Where this belongs], [As a planner-selected path for chains and cycles of reference patterns, backed by a per-basis cache. Recursive rules, when they exist, have the same structure: each fixed-point iteration multiplies a frontier by the matrix. Memory is 36 bytes per edge. The cache needs a bound and an eviction policy before it can be on by default.])

== Matrix algebra #toggle[TRIPLOX_MATRIX_ALGEBRA=1, on top of TRIPLOX_ADJ_MATRIX=1]

The CSR cache above keeps the generic join and only changes how a reference pattern answers. On `three_hop_count` the join still enumerates approximately sixteen million (a, b, c, d) tuples before it counts them. The matrix only makes each lookup cheaper, and the result is 0.88×. RedisGraph never lists the paths. It keeps one number per vertex and updates all of them at once, one hop at a time. A second toggle adds that path for the query shapes where it provably gives the same answer as the join.

#let counter-graph(ox, oy, counters, hl: ()) = {
  let k = 1.08
  let P = (a: (ox + 0.0, oy + 1.2 * k), b: (ox + 1.4 * k, oy + 2.1 * k), c: (ox + 2.8 * k, oy + 1.2 * k), d: (ox + 1.4 * k, oy + 0.3 * k), e: (ox - 1.4 * k, oy + 2.0 * k))
  let ids = (a: 1, b: 2, c: 3, d: 4, e: 5)
  for (u, v) in (("a", "b"), ("a", "c"), ("b", "c"), ("b", "d"), ("c", "d"), ("d", "a"), ("e", "a")) {
    let (ax, ay) = P.at(u); let (bx, by) = P.at(v)
    let dx = bx - ax; let dy = by - ay; let l = calc.sqrt(dx * dx + dy * dy); let ux = dx / l; let uy = dy / l
    let col = if (u, v) in hl { rust } else { accent-mid }
    draw.line((ax + ux * 0.4, ay + uy * 0.4), (bx - ux * 0.44, by - uy * 0.44), stroke: (paint: col, thickness: if (u, v) in hl { 1.2pt } else { 0.8pt }), mark: (end: "straight", scale: 0.6, fill: col))
  }
  for (kk, p) in P {
    let c = counters.at(ids.at(kk) - 1)
    draw.circle(p, radius: 0.36, fill: if ids.at(kk) == 1 and hl.len() > 0 { rust-soft } else { accent-soft }, stroke: 0.7pt + accent)
    draw.content(p, text(font: "Source Sans 3", size: 10pt, weight: 600, fill: ink, str(c)))
    draw.content((p.at(0), p.at(1) - 0.56), text(font: "Source Code Pro", size: 7pt, fill: ink2, "v" + str(ids.at(kk))))
  }
}
#fig(canvas(length: 1cm, {
  counter-graph(1.7, 0.9, (1, 1, 1, 1, 1))
  draw.content((3.2, 4.2), stext([start: every counter is 1], size: 7.6pt, fill: ink))
  arrow((5.2, 2.2), (5.9, 2.2))
  counter-graph(7.6, 0.9, (2, 2, 1, 1, 1), hl: (("d", "a"), ("e", "a")))
  draw.content((9.1, 4.45), stext([hop 1: each vertex adds up the counters], size: 7.6pt, fill: ink))
  draw.content((9.1, 4.15), stext([of the vertices that point at it], size: 7.6pt, fill: ink))
  arrow((11.1, 2.2), (11.8, 2.2))
  counter-graph(13.5, 0.9, (2, 2, 3, 2, 1))
  draw.content((15.0, 4.45), stext([hop 2: the same step again], size: 7.6pt, fill: ink))
  draw.content((15.0, 4.15), stext([2+2+3+2+1 \= 10 two-hop paths], size: 7.6pt, fill: accent))
  draw.content((9.1, 0.2), stext([v4 and v5 point at v1, so v1's new counter is 1 + 1 \= 2], size: 7.2pt, fill: rust))
}), [`two_hop_count` with one counter per vertex. The join answers the same query by listing all ten (a, b, c) paths.])

The hop in figure 17 is a matrix operation. Figure 18 shows the same hop on the grid from section 5.2.

#fig(canvas(length: 1cm, {
  let cs = 0.62; let mx = 0.6; let my = 3.9
  let ones = ((1, 2), (1, 3), (2, 3), (2, 4), (3, 4), (4, 1), (5, 1))
  draw.content((mx + 2.5 * cs + 0.4, my + 0.55), stext([A: rows = from, columns = to], size: 7.4pt, fill: ink))
  for j in range(1, 6) { draw.content((mx + 0.4 + (j - 0.5) * cs, my + 0.05), ctext("v" + str(j), size: 6.8pt)) }
  for i in range(1, 6) {
    draw.content((mx + 0.05, my - (i - 0.5) * cs), ctext("v" + str(i), size: 6.8pt))
    for j in range(1, 6) {
      let one = (i, j) in ones
      draw.rect((mx + 0.4 + (j - 1) * cs, my - i * cs), (mx + 0.4 + j * cs, my - (i - 1) * cs), fill: if j == 1 and one { rust-soft } else if j == 1 { rgb("#f6efec") } else if one { accent-soft } else { white }, stroke: 0.5pt + rule)
      draw.content((mx + 0.4 + (j - 0.5) * cs, my - (i - 0.5) * cs), ctext(if one { "1" } else { "·" }, size: 7.4pt))
    }
  }
  // counters column to the left of the grid? draw as a column vector right of grid
  let vx = mx + 0.4 + 5 * cs + 0.9
  draw.content((vx + 0.5 * cs, my + 0.55), stext([counters], size: 7.4pt, fill: ink))
  for i in range(1, 6) {
    draw.rect((vx, my - i * cs), (vx + cs, my - (i - 1) * cs), fill: if i >= 4 { rust-soft } else { white }, stroke: 0.5pt + rule)
    draw.content((vx + 0.5 * cs, my - (i - 0.5) * cs), ctext("1", size: 7.4pt))
  }
  let tx = vx + cs + 0.7
  draw.content((tx, my - 0.5 * cs), anchor: "west", stext([new counter of v1], size: 7.6pt, fill: ink))
  draw.content((tx, my - 1.4 * cs), anchor: "west", stext([\= the counters in the rows where column v1 has a 1], size: 7.4pt))
  draw.content((tx, my - 2.3 * cs), anchor: "west", stext([\= counter(v4) + counter(v5) \= 1 + 1 \= 2], size: 7.4pt, fill: rust))
  draw.content((tx, my - 3.4 * cs), anchor: "west", stext([one hop \= this sum for every column], size: 7.6pt, fill: ink))
  draw.content((tx, my - 4.3 * cs), anchor: "west", stext([new counters: 2 2 1 1 1. Only the 7 cells with a 1 do any work.], size: 7.4pt))
}), [Hop 1 as a matrix operation, for the highlighted column.])

The vocabulary for the two figures:

/ Adjacency matrix: the grid A. A 1 at row i, column j means an edge from i to j.
/ Matrix–vector product: one hop. Each new counter is a sum over one column of A. A is mostly zeros, so the product is #term[sparse] and costs one addition per edge.
/ 1ᵀ·A·A·A·1: the three-hop count written out. Start with all ones, multiply by A three times, sum.
/ Semiring: the rule for what "add" and "multiply" mean. `count` uses integers with + and ×. `count-distinct` only needs yes or no per vertex, so it uses "or" and "and".
/ Bit kernels: a row of 2000 yes/no values fits in 250 bytes, and the processor combines 128 of those bits in one instruction#footnote[SIMD, single instruction, multiple data. NEON is the SIMD instruction set on Arm processors, including the one this benchmark ran on.]. The dense yes/no matrix is capped at 64 MB, and the sorted-list form takes over above that.
/ GraphBLAS: a library interface for sparse products with a pluggable semiring#footnote[RedisGraph translated Cypher patterns into GraphBLAS calls. The prototype here writes the same operations directly in Rust over the CSR matrices of section 5.3.].

The prototype accepts a chain of reference patterns, `[?x0 :a ?x1] [?x1 :a ?x2] …`, with a single `count` or `count-distinct` in the find clause. The variables in the middle of the chain must not be used anywhere else in the same query. After a hop, a counter has merged every path that ends at its vertex, and the middle vertex of each path is lost. That is harmless when only the final count matters and wrong otherwise, so those queries run through the join.

The chain can start or end at a set fixed by other clauses, such as `[?a :g/id 42]`. That set comes from a small sub-query through the normal engine. The second shape is the triangle count, `[?a :a ?b] [?b :a ?c] [?a :a ?c]` with a `count`, answered by combining the two-hop counters with the direct edges. Everything else returns "not handled" and runs as before. An equivalence test compares the two answers on three random graphs with retractions, at head and at an as-of basis: 17 queries asserted handled and equal, 14 asserted declined.

#stats-chart("sparse-algebra")

#verdict(
  [Correctness], [Row counts identical across off, matrices and algebra arms for all 13 queries. The equivalence test covers 31 queries, including retractions and as-of bases. 648 tests pass with both toggles off, with the matrices alone, and with both on. Clippy and fmt are clean.],
  [Reading the chart], [The three aggregate-only queries the algebra handles fall by three to four orders of magnitude: `three_hop_count` 6.8 s to 0.9 ms, `three_hop_count_distinct` 7.3 s to 0.1 ms, `triangle_count` 600 ms to 1.1 ms, `two_hop_count` 238 ms to 0.6 ms. Every other query is unchanged from the matrices arm, as it must be: the tuple-returning `triangles` still enumerates its 7,821 rows.],
  [What it does not do], [Queries that return tuples. Chains whose middle variables are used elsewhere, or that use a variable twice. Aggregates other than count. Queries with inputs, `:with`, ordering or a limit. Recursive rules do not exist in Triplox yet. When they do, each fixed-point step is the same product.],
  [Cost], [One yes/no bit per pair of vertices, in each direction: 0.5 MB at 2000 vertices, capped at 64 MB. The CSR cache and its per-basis invalidation apply unchanged.])

// ==================================================================
#pagebreak()
= Zone maps #toggle[exp/zone-maps · TRIPLOX_ZONE_MAPS=1]

== Where the time goes today

A range predicate such as `[(> ?w 900)]` runs after the scan. The pattern `[?e :g/weight ?w]` reads every weight datom, and the predicate discards 90% of them. An as-of query at an old basis reads every version of every key in its range and discards the ones written later. The index is sorted by key, and a query can skip key ranges. It has no summary of the #emph[values] or #emph[transaction ids] inside a key range, and it cannot skip on those.

== The idea: minimum and maximum per block

DuckDB, and most column stores, keep the minimum and maximum of every block of a column. A predicate is checked against the summary first. If the block's maximum is below the predicate's lower bound, the block cannot contain a match, and the reader skips it. The summary is two values per block. It costs nothing when it does not help. Netezza introduced the technique under the name #term[zone map], and Parquet stores the same statistics per row group.

Zone maps only work when the values in a block are clustered. If every block spans the full range of values, no block can be skipped. Column stores sort or partition data by the columns they expect predicates on for this reason.

== Mapping onto the indexes

For each (index, attribute) prefix, the keys in key order are split into runs of 256. Each run records the minimum and maximum of the component outside the sort prefix, V for AEV and E for AVE. It also records the oldest and newest transaction id. The maps are built in memory by a scan of the prefix and cached per basis. A map built at basis #emph[S] stays valid for any query at a basis ≤ #emph[S]. Keys are never deleted, and every later key carries a transaction id greater than #emph[S].

The planner pushes comparison predicates on a variable bound by exactly one pattern into that pattern's scan as value bounds. The temporal filter consults the transaction range and skips runs that are entirely newer than the query's basis.

#fig(canvas(length: 1cm, {
  let cw = 0.68; let x0 = 0.0
  let W = ("7", "42", "60", "88", "120", "135", "190", "240", "500", "560", "610", "700", "910", "930", "960", "999")
  let S = ("910", "42", "60", "88", "120", "930", "190", "240", "500", "960", "610", "700", "7", "135", "560", "999")
  let T = ("2", "2", "3", "3", "4", "4", "5", "5", "6", "6", "7", "7", "8", "8", "9", "9")
  let keys(y, vals, faded, matchfn) = {
    for (i, v) in vals.enumerate() {
      let x = x0 + i * cw
      let m = matchfn(v)
      let f = faded.at(i)
      draw.rect((x, y), (x + cw, y + 0.42), fill: if f { panel-bg } else if m { accent-soft } else { white }, stroke: 0.5pt + (if f { rule-soft } else if m { accent } else { rule }))
      draw.content((x + cw / 2, y + 0.21), ctext(v, size: 6.6pt, fill: if f { muted } else { ink }))
      if not f { draw.line((x + 0.08, y - 0.08), (x + cw - 0.08, y - 0.08), stroke: 1.2pt + rust) }
    }
  }
  let cards(y, vals, keepfn, fmt) = {
    for r in range(4) {
      let x = x0 + r * 4 * cw
      let seg = vals.slice(r * 4, r * 4 + 4)
      let keep = keepfn(seg)
      draw.rect((x + 0.05, y), (x + 4 * cw - 0.05, y + 0.36), fill: if keep { accent-soft } else { white }, stroke: 0.5pt + (if keep { accent } else { rule }))
      draw.content((x + 2 * cw, y + 0.18), stext(fmt(seg), size: 6.4pt, fill: if keep { accent } else { ink2 }))
      draw.line((x + 2 * cw, y), (x + 2 * cw, y - 0.12), stroke: 0.5pt + rule)
    }
  }
  let minmax(seg) = { let n = seg.map(int); "min " + str(calc.min(..n)) + " · max " + str(calc.max(..n)) }
  let anymatch(seg) = seg.map(int).any(v => v > 900)
  let side(y, body, fill: ink2) = draw.content((x0 + 16 * cw + 0.25, y), anchor: "west", stext(body, size: 6.8pt, fill: fill))
  let title(y, body) = draw.content((x0, y), anchor: "west", stext(body, size: 7.8pt, fill: ink, weight: 600))
  let note(y, body) = draw.content((x0, y), anchor: "west", stext(body, size: 6.8pt, fill: ink2))

  title(9.55, [Today: the scan reads every key and tests the predicate afterwards])
  note(9.2, [16 keys of the AEV :g/weight index, in key order. The number in each cell is the weight. Query: w > 900.])
  keys(8.5, W, (false,) * 16, v => int(v) > 900)
  side(8.71, [16 keys read, 4 match], fill: rust)
  draw.content((x0, 8.2), anchor: "west", stext([red underline = the key is read from storage · green cell = passes the predicate], size: 6.4pt, fill: muted))

  title(7.4, [With a zone map: one card per run of 4 keys holds the run's min and max, and the scan checks the card first])
  cards(6.55, W, anymatch, minmax)
  keys(5.85, W, (true,) * 12 + (false,) * 4, v => int(v) > 900)
  side(6.06, [4 keys read, 4 match], fill: accent)
  note(5.5, [Three cards say "max below 900", so their 12 keys are never read. The map itself is 8 numbers.])

  title(4.55, [The same 16 keys with the weights shuffled: every run now holds a large weight, and no run can be skipped])
  cards(3.7, S, anymatch, minmax)
  keys(3.0, S, (false,) * 16, v => int(v) > 900)
  side(3.21, [16 keys read, 4 match], fill: rust)
  note(2.65, [Zone maps only help when neighbouring keys hold similar values. In the benchmark, weight = id mod 1000 and keys are sorted by id, so they do.])

  title(1.7, [Time travel: each card also holds the oldest and newest transaction in its run. Query: as of transaction 1])
  cards(0.85, T, seg => false, seg => { let n = seg.map(int); "tx " + str(calc.min(..n)) + " to " + str(calc.max(..n)) })
  keys(0.15, T, (true,) * 16, v => false)
  side(0.36, [0 keys read], fill: accent)
  note(-0.2, [The number in each cell is the transaction that wrote the key. Every card starts after transaction 1, so the reader skips all four runs without touching a key.])
}), [Zone maps on a slice of the weight index, before and after. A card costs two numbers per run and is checked before the run is read. On the benchmark, runs are 256 keys: the value predicate skipped 1230 of 2000 seeks, and the as-of queries skipped 155 of 155 runs. The shuffled row shows the limit of the technique: the map cannot skip a run that mixes small and large values. Transaction pruning does not have this limit, because Triplox never rewrites an old key, so transaction ids always rise along the key order.])

== Walkthrough: weight_filter and the as-of queries

`weight_filter` binds ?w through exactly one pattern. The planner converts `> 900` into a lower bound on the scan of `:g/weight` in AEV. The scan consults the run summaries and skips runs whose maximum is at or below 900. On the benchmark data the weights rise with entity id, and 1230 of the 2000 per-entity seeks are pruned.

The experiment also added three as-of queries. Two read at a basis before the edges were loaded. There, the transaction range skips every run, 155 of 155, and the queries run 6 to 7× faster. The third reads at a basis in the middle of the load. Only the runs newer than that basis are skipped, and the gain is 1.4×. The first sample of `label_lookup` pays a 22 ms build of a map for `:g/label` that the query never uses. The median hides that cost, and the chart shows no effect.

== Results

#stats-chart("zone-maps")

#verdict(
  [Correctness], [647 tests pass, once normally and once with the toggle forced on for the whole suite. The equivalence test covers 10 queries × latest/as-of × cold/warm cache.],
  [Reading the chart], [Only rows with skipped runs show a real effect: `weight_filter`, `heavy_neighbors`, and the three as-of rows. `heavy_neighbors` gains 8% from the same value bound on `:g/weight`. Every other row has zero skips, and its ratio is noise.],
  [Assessment], [Transaction-range pruning is worth implementing, at approximately a week with incremental rebuild. Value pruning is not. A planner change that routes value-bounded patterns through AVE as a range scan serves the range predicate case better and needs no zone map. Counters in the experiment confirmed that the planner currently runs that query as 2000 per-entity seeks instead.],
  [Defects in the prototype], [Maps are built on every scan, including at head basis where they can never prune. The cache is unbounded, and each new basis rebuilds it with a full prefix scan.])

// ==================================================================
#pagebreak()
= Combinations

Three stacks were merged from the finished branches. Each was benchmarked with all toggles off, each toggle alone, and all toggles on. This attributes the contribution of each part instead of summing them. In every table the ratio is against the stack's own off arm. The "singles multiplied" comparison uses the single-toggle arms of the same run.

== Vectorized join and row-major segments #toggle[exp/combo-vec-seg]

The two changes attack different costs: the join's intermediate rows and the storage's per-key overhead. The merge was one import block. All-on beats the product of the two single ratios on eight of ten queries. Each feature was partly hidden behind the other. The batched engine alone still paid an LSM lookup per datom, and segments alone still allocated a row per output.

`neighbors_of_42` is the exception. A point lookup that returns 21 rows does not repay the batched engine's setup, and all-on is no better than segments alone on that query.

#stats-chart("combo-vec-seg")

#verdict(
  [Correctness], [Row counts are identical to baseline in all four arms. 647 tests pass with both toggles off and both on. A new equivalence test compares the batched and row engines over segmented storage, the pairing neither branch covered.],
  [Interaction], [`three_hop_count` gains nothing from segments alone, 1.00× with an interval that includes 1. On top of the batched engine the same segments give a further 14%, from 0.29× to 0.25×. The deep join is bound by row bookkeeping, not by key count, until the bookkeeping is removed.])

== Vectorized join and columnar segments #toggle[exp/combo-vec-col]

This stack answers the question left open in chapter 4. Columnar segments alone gave `three_hop_count` nothing, because each single-row seek decoded a whole segment. With the batched engine the same segments turn into a gain, from 0.28× to 0.25×. The mechanism is visible in the code, not only in the numbers.

The batched engine sorts each level's bindings by key before it extends them, originally to avoid a `BTreeMap`. The segment iterator returns at once when it is already at or past a seek target. A run of ascending seeks therefore decodes each segment once. The row engine's seek order is arbitrary, and that is the access pattern segments handle worst.

#fig(canvas(length: 1cm, {
  let panel(px, title, seeks, decode-col) = {
    draw.content((px, 3.5), anchor: "west", stext(title, size: 7.6pt, fill: ink))
    // segments
    draw.rect((px, 0.3), (px + 1.7, 1.15), fill: white, stroke: 0.6pt + accent)
    draw.content((px + 0.85, 1.32), stext([segment S1], size: 6.6pt, fill: accent))
    strip(px + 0.3, 0.45, 0.55, 0.45, ("1", "2"), fills: (accent-soft,) * 2, stroke: accent, size: 7pt)
    draw.rect((px + 2.2, 0.3), (px + 4.6, 1.15), fill: white, stroke: 0.6pt + accent)
    draw.content((px + 3.4, 1.32), stext([segment S2], size: 6.6pt, fill: accent))
    strip(px + 2.55, 0.45, 0.55, 0.45, ("3", "4", "5"), fills: (accent-soft,) * 3, stroke: accent, size: 7pt)
    let opened = ()
    let decodes = 0
    for (i, sk) in seeks.enumerate() {
      let x = px + 0.45 + i * 0.95
      draw.circle((x, 2.7), radius: 0.24, fill: white, stroke: 0.6pt + ink2)
      draw.content((x, 2.7), ctext(sk, size: 7pt))
      draw.content((x, 3.1), stext("seek " + str(i + 1), size: 6pt, fill: muted))
      let seg = if sk == "1" or sk == "2" { 1 } else { 2 }
      let tx = if seg == 1 { px + 0.85 } else { px + 3.4 }
      let fresh = decode-col == rust or seg not in opened
      if fresh { decodes = decodes + 1 }
      opened.push(seg)
      draw.line((x, 2.44), (tx, 1.5), stroke: (paint: if fresh { rust } else { accent-mid }, thickness: if fresh { 0.9pt } else { 0.6pt }, dash: if fresh { "solid" } else { "dotted" }), mark: (end: "straight", scale: 0.5, fill: if fresh { rust } else { accent-mid }))
    }
    draw.content((px, -0.15), anchor: "west", stext([decodes: ] + str(decodes) + [ for 5 seeks], size: 7.2pt, fill: if decodes > 2 { rust } else { accent }))
  }
  panel(0, [row engine: seeks in binding order], ("3", "1", "4", "2", "5"), rust)
  panel(6.6, [batched engine: bindings sorted first], ("1", "2", "3", "4", "5"), accent)
  draw.content((0, -0.6), anchor: "west", stext([solid arrow: the segment is decoded · dotted arrow: the iterator is already inside this segment and only advances], size: 6.6pt, fill: ink2))
}), [Why columnar segments help the deep join only under the batched engine. Each seek lands in the segment that owns its entity. Unsorted seeks bounce between segments and decode one per seek. Sorted seeks decode each segment once and advance within it. With 256-datom segments and about 20 seeks per segment, the decode cost per seek falls by an order of magnitude.])

#stats-chart("combo-vec-col")

#verdict(
  [Correctness], [Row counts are identical to baseline in all four arms. With toggles off, and with the batched join alone, all tests pass. Any configuration with the columnar layout fails 11 library tests: the 10 documented in chapter 4 plus the batched-versus-row equivalence test. That test seeds SlateDB with raw row keys and cannot run over segments. Under the columnar layout no test asserts that the two engines return identical rows. The matching bench row counts are the only evidence for that combination.],
  [Interaction], [Gains more than compose on every multi-level query: triangles 0.39× against a product of the two single ratios of 0.47×, `two_hop_count` 0.32× against 0.44×. The merge added no new code. The batched engine reads storage through the same two helper functions as the row engine, and a layout swap under them was sufficient.])

== The full stack #toggle[exp/combo-full]

This stack combines the vectorized join, columnar segments and adjacency matrices, and a sixth arm adds the matrix algebra of section 5.5 on top. The question is whether the matrices still pay once scans are cheap and the join is batched. The chart has six arms: everything off, each of the three toggles alone, the three together, and the three together plus the algebra toggle. The algebra is opt-in, so the "join + columnar + matrices" arm runs every query through the join, and the last arm differs from it only on the queries the algebra recognises.



#stats-chart("combo-full-algebra")

#verdict(
  [Correctness], [Row counts are identical to baseline in every arm. Off, batched, adjacency, and adjacency with batched: all tests pass. Any configuration with the columnar layout fails the same 11 harness-artifact tests as the previous stack, no more. With the algebra merged in, the row layout passes 657 tests with all four toggles on, and the columnar layout fails the same 11.],
  [Does the matrix still pay?], [Yes. Columnar alone brings triangles to 0.78× and the batched join to 0.57×. The matrix alone brings it to 0.03×, and all three together to 0.02×, 12 ms against 563 ms. Cheaper scans and a faster join shrink the constant the matrix saves. But the matrix replaces a probe per candidate with one intersection, and no storage or layout change substitutes for that.],
  [Which part does what], [On the scans, `out_degree` and `in_degree_top`, the matrix gives 0.15× and the full stack 0.07×: the matrix answers from its offsets array, and columnar segments make building it cheaper. On the scalar queries the matrix does nothing, as it must, and the whole gain is the segments. On `three_hop_count` the batched join does all the work among the first three, 0.29× alone against 0.26× for the stack.],
  [With the algebra], [The algebra arm changes only the aggregate-only queries, and changes them completely: `three_hop_count` 6.6 s to 0.86 ms, `three_hop_count_distinct` 7.2 s to 0.11 ms, `two_hop_count` 235 ms to 0.59 ms, `triangle_count` 606 ms to 1.07 ms. Every other query is within noise of the all-three arm. The merge into this stack needed no code change beyond three conflict resolutions, and the row counts match across all six arms.],
  [Integration], [Merging the three prototypes exposed three interactions between their defaults, all fixed on the branch and listed in its COMBINED.md. One built a wrong matrix with no error. The adjacency equivalence test caught it.])

// ==================================================================
#pagebreak()
#pagebreak()
= Defects found

None of these was the object of an experiment. Each is fixed on the branch that met it and deserves an issue against main.

+ *Proposer selection ignores cost on uncompacted data.* The generic join asks each pattern that can bind a variable for an estimated candidate count. The smallest estimate proposes. The estimate comes from SlateDB range statistics. These return 0 for every prefix while data is resident in the memtable or write-ahead log. With every estimate at 0, ties go to pattern order. The defect surfaced when the adjacency matrix reported an exact count of 7 to 37 and lost to an estimate of 0. The `:g/weight` pattern then proposed all 2000 entities per row and materialized four million rows for `heavy_neighbors`. Main has this defect today on any dataset that has not been compacted.
+ *Integer index keys sort in descending order.* The codec encodes longs as `value ^ i64::MAX`. This reverses order over the whole range, while doubles and strings sort ascending. The comment in the codec describes the encoding as order-preserving. No current query path depends on ascending order. A value range scan on AVE, which chapter 6 recommends, must account for the direction.
+ *Segment rewrites replay earlier datoms through the change feed.* A segment rewrite republishes the datoms of older transactions. Any layout that stores several datoms per key does this. The incremental engine subscribes to that feed. It double-counted every replayed datom until the CDC layer was changed to keep only datoms from the newest transaction in a batch.
+ *Two split defects and one decode off-by-one* in the segment prototypes themselves. An on/off equivalence test caught each one before benchmarking. They are listed because they show what the equivalence tests are for.

#pagebreak()
= Memory, writes and the object store

The benchmark ran on an in-memory node and measured only time. Nobody measured memory use or object-store traffic. The figure below gives both for each technique, worked out from the code and from the benchmark's sizes. Only the two disk sizes marked as measured come from a run.

#let cost-chart() = canvas(length: 1cm, {
  let rows = ("current engine", "vectorized join", "datoms per key", "columnar segments", "adjacency matrices", "matrix algebra", "zone maps")
  let rh = 0.4; let x0 = 3.3; let span = 9.6; let ph = rows.len() * rh + 1.35
  let bytes(v) = if v >= 1e9 { str(calc.round(v / 1e9, digits: 1)) + " GB" } else if v >= 1e6 { str(calc.round(v / 1e6, digits: 2)) + " MB" } else if v >= 1e3 { str(calc.round(v / 1e3)) + " KB" } else { str(calc.round(v)) + " B" }
  let count(v) = if v >= 1e3 { str(calc.round(v / 1e3, digits: 1)) + " k" } else { str(calc.round(v)) }
  let panels = (
    ([Memory held while idle], [bytes, at the benchmark's size], 1e3, 1e9, ((none, false, [none]), (none, true, [none kept. 256 MB peak during three_hop_count, freed at the end]), (none, false, [none. 7 KB decode buffer per read]), (none, false, [none. 4 KB decode buffer per read]), (1.45e6, false, [36 B per edge, per attribute and basis. No bound]), (5e5, false, [a dense bit matrix per orientation, n² bits, capped at 64 MB]), (1e4, false, [per index, attribute and basis. No bound])), bytes),
    ([Bytes rewritten in storage when one edge datom arrives], [six indexes], 1e2, 1e5, ((162, false, [one 27 B key in each index]), (162, false, [unchanged]), (4.1e4, false, [a 256-datom segment in each index]), (8.3e3, false, [two compressed 1024-datom segments and two row keys]), (162, false, [unchanged. The write invalidates the cache]), (162, false, [unchanged, same invalidation]), (162, false, [unchanged. The write invalidates the cache])), bytes),
    ([Storage blocks fetched for one full scan of :g/to], [4 KB blocks. On an object store each block is one request], 1, 1e3, ((260, false, [39,764 keys, approximately 1 MB]), (260, false, [unchanged]), (280, false, [the same bytes plus 8%. Fewer keys, not fewer blocks]), (100, false, [61% fewer bytes]), (none, false, [none while the cache is warm. Each new basis rebuilds it from all 260 blocks]), (none, false, [none, same cache]), (260, false, [unchanged, and each new basis rebuilds the map from all 260 blocks])), count),
  )
  for (pi, (title, unit, lmin, lmax, vals, fmt)) in panels.enumerate() {
    let ytop = -pi * ph
    let lx(v) = x0 + 1.0 + (calc.log(v, base: 10) - calc.log(lmin, base: 10)) / (calc.log(lmax, base: 10) - calc.log(lmin, base: 10)) * (span - 1.0)
    draw.content((0, ytop), anchor: "west", stext(title, size: 7.8pt, fill: ink, weight: 600))
    draw.content((0, ytop - 0.28), anchor: "west", stext(unit, size: 6.4pt, fill: muted))
    let yax = ytop - 0.62
    draw.line((x0, yax), (x0 + span, yax), stroke: 0.4pt + rule)
    draw.content((x0 + 0.12, yax - 0.06), anchor: "north", stext([none], size: 5.8pt, fill: muted))
    let t = lmin
    while t <= lmax { draw.line((lx(t), yax), (lx(t), yax - 0.07), stroke: 0.4pt + rule); draw.content((lx(t), yax - 0.1), anchor: "north", stext(fmt(t), size: 5.8pt, fill: muted)); t = t * 10 }
    for (ri, (v, hollow, note)) in vals.enumerate() {
      let y = yax - 0.55 - ri * rh
      draw.content((x0 - 0.2, y), anchor: "east", stext(rows.at(ri), size: 7pt, fill: if ri == 0 { ink2 } else { ink }))
      let x = if v == none { x0 + 0.12 } else { lx(calc.max(v, lmin)) }
      let col = if ri == 0 { ink2 } else if v == none and not hollow { muted } else { accent }
      draw.line((x0, y), (x0 + span, y), stroke: 0.25pt + rule-soft)
      draw.circle((x, y), radius: 0.075, fill: if hollow { white } else { col }, stroke: 0.7pt + col)
      if x < x0 + span * 0.55 { draw.content((x + 0.15, y + 0.01), anchor: "west", stext(note, size: 6pt, fill: ink2)) } else { draw.content((x - 0.15, y + 0.01), anchor: "east", stext(note, size: 6pt, fill: ink2)) }
    }
  }
})

#fig(cost-chart(), [Block counts assume SlateDB's 4 KB blocks and are estimates. Hollow marker: a short-lived peak, not memory that stays allocated.])

Three things follow for an object store, where every block is a request that costs tens of milliseconds and a fraction of a cent. The key-count reduction of chapter 3 is a CPU gain, not a fetch gain: the same bytes sit in the same number of blocks. Only the byte reduction of the columnar layout fetches fewer blocks, and both layouts write more, at roughly twelve times the price per request that a read costs. The vectorized join saves CPU and nothing else, which is a smaller share of a query that waits on the network. And the caches of chapters 5 and 6 are the strongest lever or the worst cost, depending on one thing: a warm matrix answers a traversal with zero fetches, while a rebuild per basis reads every block of the attribute. On an object store they pay only if the change feed maintains them.

#pagebreak()
= Conclusions

#let tldr(..items) = {
  let rows = ()
  for (k, (title, body)) in items.pos().enumerate() {
    rows.push(align(right + top, text(font: "Source Sans 3", size: 22pt, weight: 300, fill: accent-mid, str(k + 1))))
    rows.push({ text(font: "Source Sans 3", size: 11pt, weight: 600, fill: ink, title); v(3pt); text(size: 10pt, body) })
  }
  grid(columns: (28pt, 1fr), column-gutter: 14pt, row-gutter: 14pt, ..rows)
}

#v(6pt)
Do these, in this order. Against an object store, items 2 and 3 swap: a cache that removes fetches is worth more than a layout that shrinks them, once the change feed maintains it.
#v(10pt)

#tldr(
  ([Vectorized join], [Every query with more than one pattern runs 1.1× to 3.5× faster. It touches the query engine and nothing else.]),
  ([Segments, columnar], [Many datoms per key makes scans and point lookups 3× to 22× faster in memory. Against an object store only the columnar layout's 61% byte reduction fetches fewer blocks, and both layouts write more, so build the columnar one and measure the write cost first. It alters the storage format and what the change feed carries, so design it before writing it, and give point lookups a per-segment index.]),
  ([Matrices and algebra], [Aggregate-only graph counts fall from seconds to a millisecond, and a warm matrix answers a traversal with no storage fetch at all, which is the largest possible gain against an object store. Worth it only if graph queries exist, and only once the matrices are fed from the change feed: rebuilt per basis, they read the whole attribute per transaction and cost more than they save.]),
  ([Zone maps], [1.4× to 7× on time-travel queries, nothing elsewhere. Value pruning depends on the data, not on Triplox.]),
)

#v(14pt)
#line(length: 100%, stroke: 0.5pt + rule)
#v(6pt)

The benchmark flatters items 3 and 4. It is one uniform integer graph with one reference attribute, and seven of its ten queries traverse that attribute. Real workloads are mostly strings, small tables and scalar expressions, with few joins. Items 1 and 2 cut costs every query pays; the others need a realistic workload before they are worth more than an experiment, and string handling was not measured at all.

Two defects in main are harmless today and stop being harmless later. The planner's estimate of zero on uncompacted data makes plans follow clause order, which fails the moment one pattern reports a true count, as the matrices do. Integer keys sort in descending order, which fails the moment a value-range scan on AVE exists.

Wide scalar scans stay far from column-store speed. After every technique, `weight_sum` still spends approximately 0.3 µs per value. That comparison is an estimate: no DuckDB run was made.

#pagebreak()
= Appendix: full statistics

Each table lists, per query, the sample count and the median of each arm with its 95% bootstrap interval. It then lists the ratio of medians with its interval and the two-sided Mann-Whitney p-value. Combination stacks list the ratio of each arm to the off arm.

#let stats-or-pending(key) = if key in stats { stats-table(key) } else { pending }
== Vectorized join
#stats-or-pending("vectorized-join")
== Multiple datoms per key
#stats-or-pending("datom-segments")
== Columnar segments
#stats-or-pending("columnar-storage")
== Sparse adjacency matrices
#stats-or-pending("sparse-matrix")
== Matrix algebra
#stats-or-pending("sparse-algebra")
== Zone maps
#stats-or-pending("zone-maps")
== Vectorized join and row-major segments
#stats-or-pending("combo-vec-seg")
== Vectorized join and columnar segments
#stats-or-pending("combo-vec-col")
== The full stack
#stats-or-pending("combo-full-algebra")

#v(6pt)
#toggle[Each experiment branch contains an EXPERIMENT.md with hook points, negative results and integration cost, and a PROGRESS.md hand-off. Result JSON for every A/B run is retained alongside this report.]
