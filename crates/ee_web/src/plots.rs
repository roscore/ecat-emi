//! Canvas 2D plots: spectrum, MLT-3 eye sketch, PWM burst timing.
use web_sys::CanvasRenderingContext2d as C2;

const AXIS: &str = "#5a6676";
const TEXT: &str = "#aab6c4";

fn clear(c: &C2, w: f64, h: f64) {
    c.clear_rect(0.0, 0.0, w, h);
    c.set_font("11px system-ui, sans-serif");
    c.set_line_width(1.0);
}

fn text(c: &C2, s: &str, x: f64, y: f64, col: &str) {
    c.set_fill_style_str(col);
    let _ = c.fill_text(s, x, y);
}

/// Log-log spectrum (V/Hz, shown in dB) with the receiver passband shaded.
pub fn spectrum(c: &C2, w: f64, h: f64, freqs: &[f64], curves: &[(&[f64], &str, &str)], rx_bw: f64, band_label: &str) {
    clear(c, w, h);
    let (l, r, t, b) = (34.0, w - 6.0, 8.0, h - 18.0);
    let (f0, f1) = (freqs.first().copied().unwrap_or(1e4), freqs.last().copied().unwrap_or(1e9));
    let fx = |f: f64| l + (f / f0).ln() / (f1 / f0).ln() * (r - l);
    let db = |v: f64| 20.0 * v.max(1e-30).log10();
    let top = curves.iter().flat_map(|(v, _, _)| v.iter()).map(|&v| db(v)).fold(-300.0, f64::max).ceil() + 5.0;
    let lo = top - 160.0;
    let fy = |v: f64| t + (top - db(v).max(lo)) / (top - lo) * (b - t);
    c.set_fill_style_str("#1d3a2a");
    c.fill_rect(l, t, fx(rx_bw.min(f1)) - l, b - t);
    text(c, band_label, l + 3.0, t + 11.0, "#7fd19a");
    c.set_stroke_style_str(AXIS);
    c.stroke_rect(l, t, r - l, b - t);
    for d in 4..=9 {
        let x = fx(10f64.powi(d));
        text(c, &format!("1e{d}"), x - 9.0, h - 5.0, TEXT);
    }
    for k in 0..=4 {
        let v = top - k as f64 * 40.0;
        text(c, &format!("{v:.0}"), 2.0, t + k as f64 * (b - t) / 4.0 + 4.0, TEXT);
    }
    for (i, (vals, col, label)) in curves.iter().enumerate() {
        c.set_stroke_style_str(col);
        c.set_line_width(if i == 0 { 1.0 } else { 1.6 });
        c.begin_path();
        for (k, (&f, &v)) in freqs.iter().zip(vals.iter()).enumerate() {
            if k == 0 { c.move_to(fx(f), fy(v)) } else { c.line_to(fx(f), fy(v)) }
        }
        c.stroke();
        text(c, label, r - 130.0, t + 12.0 + 12.0 * i as f64, col);
    }
    text(c, "dBV/Hz", l + 3.0, b - 4.0, TEXT);
}

/// MLT-3 eye (levels −A, 0, +A) with the noise peak drawn as a band around each level.
pub fn eye(c: &C2, w: f64, h: f64, eye_rx: f64, peak: f64, labels: [&str; 3]) {
    clear(c, w, h);
    let a = eye_rx / 2.0;
    let span = (a + peak).max(a * 1.2) * 1.15;
    let y = |v: f64| h / 2.0 - v / span * (h / 2.0 - 10.0);
    let (x0, x1) = (10.0, w - 10.0);
    c.set_fill_style_str("rgba(255,80,80,0.25)");
    for lv in [-a, 0.0, a] {
        c.fill_rect(x0, y(lv + peak), x1 - x0, y(lv - peak) - y(lv + peak));
    }
    c.set_stroke_style_str("#6aa9ff");
    c.set_line_width(1.4);
    let ui = (x1 - x0) / 2.0;
    for (s, e) in [(-a, 0.0), (0.0, a), (a, 0.0), (0.0, -a), (-a, -a), (0.0, 0.0), (a, a)] {
        c.begin_path();
        for k in 0..=24 {
            let t = k as f64 / 24.0;
            let v = s + (e - s) * (0.5 - 0.5 * (std::f64::consts::PI * t).cos());
            let x = x0 + ui * 0.5 + t * ui;
            if k == 0 { c.move_to(x, y(v)) } else { c.line_to(x, y(v)) }
        }
        c.stroke();
    }
    let m = a / 2.0 - peak;
    text(c, &format!("{} {:.0} mV p-p · {} {:.1} mV", labels[0], eye_rx * 1e3, labels[1], peak * 1e3), 12.0, 13.0, TEXT);
    text(c, &format!("{} {:.1} mV", labels[2], m * 1e3), 12.0, h - 5.0, if m > 0.0 { "#7fd19a" } else { "#ff8a80" });
}

/// One PWM period: bursts at both edges and one frame window.
#[allow(clippy::too_many_arguments)]
pub fn burst(c: &C2, w: f64, h: f64, f_pwm: f64, duty: f64, burst_s: f64, frame_s: f64, labels: [&str; 4], exposure: f64, frame_hit: f64) {
    clear(c, w, h);
    let period = 1.0 / f_pwm;
    let (l, r) = (10.0, w - 10.0);
    let x = |t: f64| l + t / period * (r - l);
    c.set_stroke_style_str(AXIS);
    c.begin_path();
    c.move_to(l, 60.0);
    c.line_to(r, 60.0);
    c.stroke();
    // PWM waveform.
    c.set_stroke_style_str("#ff8c1a");
    c.begin_path();
    c.move_to(x(0.0), 50.0);
    c.line_to(x(duty * period), 50.0);
    c.line_to(x(duty * period), 25.0);
    c.line_to(x(period), 25.0);
    c.stroke();
    // Bursts (at least 2 px wide so they stay visible).
    c.set_fill_style_str("rgba(255,80,80,0.8)");
    for t in [0.0, duty * period] {
        c.fill_rect(x(t), 20.0, (x(burst_s) - l).max(2.0), 40.0);
    }
    c.set_fill_style_str("rgba(106,169,255,0.5)");
    let fs = 0.3 * period;
    c.fill_rect(x(fs), 64.0, (x(frame_s) - l).max(2.0), 12.0);
    text(c, labels[0], x(fs), 88.0, "#6aa9ff");
    text(c, &format!("{} {:.1} µs", labels[1], period * 1e6), r - 120.0, 14.0, TEXT);
    text(c, &format!("{} {:.2} % · {} {:.1} %", labels[2], exposure * 100.0, labels[3], frame_hit * 100.0), l, h - 6.0, TEXT);
}
