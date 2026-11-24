//! KO / EN strings (Korean default).
use ee_core::couple::{FixKind, Path};

/// Keys built in code (not present as data-t in index.html).
pub const DYNAMIC: &[&str] = &["line", "err_title", "pairs", "aggressors", "side_phy", "side_line", "kind_sw", "kind_gate", "kind_bus", "tag_aggr", "tag_p", "tag_n", "selected", "ber", "fer", "lost", "dominant", "margin", "fixes", "no_fix", "ok", "marginal", "fail", "len_l", "spacing", "height", "contrib", "gerber_layers", "g_ignore", "board_info", "aggr_spec", "rx_band", "noise", "exposure", "frame_hit", "frame", "pwm_period", "eye_rx", "peak", "mode_conv", "skew", "hot_loop", "no_pairs", "gaps", "gerber_note"];

/// (key, Korean, English)
const T: &[(&str, &str, &str)] = &[
    ("subtitle", "보드 레벨 EtherCAT EMI 커플링 추정", "board-level EtherCAT EMI coupling estimate"),
    ("privacy", "🔒 파일은 브라우저 밖으로 나가지 않습니다 (업로드·저장 없음)", "🔒 Your file never leaves the browser (no upload, nothing stored)"),
    ("h_load", "파일 불러오기", "Load a board"),
    ("sample", "샘플 보드 불러오기", "Load sample board"),
    ("drop", ".kicad_pcb, Gerber/Excellon 파일 또는 .zip 을 선택하거나 보드 화면에 끌어다 놓으세요.", "Pick a .kicad_pcb, Gerber/Excellon files or a .zip, or drop them on the board view."),
    ("pick_hint", "보드에서 넷을 클릭하면 역할을 지정할 수 있습니다.", "Click a net on the board to tag its role."),
    ("h_roles", "넷 역할", "Net roles"),
    ("h_aggr", "공격원 (인버터 프리셋)", "Aggressor (inverter presets)"),
    ("k_vbus", "버스 전압 V", "Bus voltage V"),
    ("k_tr", "상승 시간 ns", "Rise time ns"),
    ("k_iph", "상 전류 A", "Phase current A"),
    ("k_fpwm", "PWM 주파수 kHz", "PWM frequency kHz"),
    ("k_csw", "스위치 노드 C pF", "Switch-node C pF"),
    ("preset_hint", "변경 시 모든 공격원 넷의 dV / dI / tr 프리셋이 다시 적용됩니다.", "Changing these re-applies the dV / dI / tr presets to every aggressor net."),
    ("h_link", "피해자 (100BASE-TX 링크)", "Victim (100BASE-TX link)"),
    ("k_lcl", "모드 변환 LCL dB", "Mode conversion LCL dB"),
    ("k_cmrr", "마그네틱 CMRR dB", "Magnetics CMRR dB"),
    ("k_len", "케이블 길이 m", "Cable length m"),
    ("k_frame", "프레임 바이트", "Frame bytes"),
    ("k_cycle", "사이클 µs", "Cycle µs"),
    ("k_floor", "수신 잡음 바닥 mV", "RX noise floor mV"),
    ("h_cable", "보드 간 케이블 (보조)", "Board-to-board cable (secondary)"),
    ("k_b2b", "케이블 길이 m", "Cable length m"),
    ("k_sep", "상 케이블과 간격 mm", "Spacing to phase cable mm"),
    ("k_shd", "실드", "Shield"),
    ("o_none", "없음", "none"),
    ("o_foil", "포일", "foil"),
    ("o_braid", "편조", "braid"),
    ("k_gnd", "실드 접지", "Shield ground"),
    ("o_one", "한쪽 끝", "one end"),
    ("o_both", "양쪽 끝", "both ends"),
    ("k_fer", "페라이트 Ω@100MHz", "Ferrite Ω@100 MHz"),
    ("empty", "보드를 불러오세요 — 또는 「샘플 보드 불러오기」", "Load a board — or press “Load sample board”"),
    ("v_iso", "등각", "Iso"),
    ("v_top", "위", "Top"),
    ("v_bottom", "아래", "Bottom"),
    ("xray", "X-ray", "X-ray"),
    ("k_exag", "두께 과장", "Thickness ×"),
    ("opacity", "불투명도", "Opacity"),
    ("reset", "편집 되돌리기", "Reset edits"),
    ("drag_hint", "위 보기에서 초록 페어를 옆으로 끌면 가상으로 이동합니다.", "In the top view, drag a green pair sideways to move it virtually."),
    ("fallback", "WebGL2 를 쓸 수 없어 2D 위 보기로 표시합니다.", "WebGL2 is not available: showing a 2D top view."),
    ("h_spec", "스펙트럼 (V/Hz)", "Spectrum (V/Hz)"),
    ("h_eye", "아이 다이어그램 (MLT-3)", "Eye diagram (MLT-3)"),
    ("h_burst", "버스트 타이밍", "Burst timing"),
    ("export_md", "보고서 Markdown", "Report Markdown"),
    ("export_csv", "커플링 CSV", "Coupling CSV"),
    ("honesty", "집중 정수, 준정적 커플링 모델입니다. 풀웨이브 전자기장 해석이 아니며 적합성 시험을 대체하지 않습니다. 설계 직관과 대안 비교용입니다.", "Lumped, quasi-static coupling model — not a full-wave field solve and not a compliance test. Use it for design intuition and for comparing options."),
    ("line", "줄", "line"),
    ("err_title", "파일을 읽을 수 없습니다", "Could not read the file"),
    ("pairs", "피해 페어 (EtherCAT)", "Victim pairs (EtherCAT)"),
    ("aggressors", "공격원 넷", "Aggressor nets"),
    ("side_phy", "PHY↔마그네틱", "PHY↔magnetics"),
    ("side_line", "마그네틱↔RJ45", "magnetics↔RJ45"),
    ("kind_sw", "스위치 노드", "switch node"),
    ("kind_gate", "게이트", "gate"),
    ("kind_bus", "버스", "bus"),
    ("tag_aggr", "공격원으로", "as aggressor"),
    ("tag_p", "페어 P", "pair P"),
    ("tag_n", "페어 N", "pair N"),
    ("selected", "선택한 넷", "Selected net"),
    ("ber", "BER", "BER"),
    ("fer", "FER", "FER"),
    ("lost", "손실 프레임/시간", "lost frames/h"),
    ("dominant", "주 경로", "dominant path"),
    ("margin", "아이 여유", "eye margin"),
    ("fixes", "대책 (단독 적용, 순위)", "Fixes (each alone, ranked)"),
    ("no_fix", "대책 불필요", "no fix needed"),
    ("ok", "양호", "OK"),
    ("marginal", "주의", "Marginal"),
    ("fail", "불합격", "Fail"),
    ("len_l", "ℓ 길이", "ℓ length"),
    ("spacing", "s 간격", "s spacing"),
    ("height", "h 기준면 높이", "h to reference"),
    ("contrib", "기여", "contribution"),
    ("gerber_layers", "Gerber 레이어 (추정 — 수정 가능)", "Gerber layers (guessed — editable)"),
    ("g_ignore", "무시", "ignore"),
    ("board_info", "넷 {n} · 트랙 {s} · 비아 {v} · 레이어 {l} · 두께 {t} mm", "{n} nets · {s} tracks · {v} vias · {l} layers · {t} mm thick"),
    ("aggr_spec", "공격원 스텝", "aggressor step"),
    ("rx_band", "수신 대역", "RX band"),
    ("noise", "수신단 잡음", "noise at RX"),
    ("exposure", "버스트 노출", "burst exposure"),
    ("frame_hit", "프레임 피격 확률", "frame hit chance"),
    ("frame", "프레임", "frame"),
    ("pwm_period", "PWM 주기", "PWM period"),
    ("eye_rx", "RX 아이", "RX eye"),
    ("peak", "잡음 피크", "noise peak"),
    ("mode_conv", "모드 변환 @31 MHz", "mode conversion @31 MHz"),
    ("skew", "P/N 길이 차", "P/N skew"),
    ("hot_loop", "핫 루프", "hot loop"),
    ("no_pairs", "피해 페어가 없습니다. 넷을 클릭해 P / N 으로 지정하세요.", "No victim pairs. Click nets and tag them P / N."),
    ("gaps", "평면 틈 통과", "plane-gap crossings"),
    ("gerber_note", "Gerber 에는 넷이 없어 구리 섬(ISL) 단위로 표시합니다. 두께 1.6 mm 가정.", "Gerber has no nets: copper islands (ISL) are shown. 1.6 mm thickness assumed."),
];

pub fn tr(ko: bool, k: &str) -> &str {
    T.iter().find(|e| e.0 == k).map(|e| if ko { e.1 } else { e.2 }).unwrap_or(k)
}

pub fn fix_text(ko: bool, f: &FixKind, pair: &str) -> String {
    if !ko {
        return ee_core::report::fix_str(f, pair);
    }
    match f {
        FixKind::MovePair { mm } => format!("페어 {pair} 를 {mm:.1} mm 이동"),
        FixKind::FillGap { at } => format!("({:.1}, {:.1}) mm 의 평면 틈 메우기", at[0], at[1]),
        FixKind::ShrinkHotLoop { mm2 } => format!("핫 루프를 {mm2:.1} mm² 로 축소"),
        FixKind::StitchVias { at } => format!("({:.1}, {:.1}) mm 주변 스티칭 비아 추가", at[0], at[1]),
        FixKind::SlowEdges { t_r } => format!("엣지를 {:.0} ns 로 완화", t_r * 1e9),
        FixKind::InnerLayer => format!("{pair} 를 평면 사이 내층으로 배선"),
    }
}

pub fn path_text(ko: bool, p: Path) -> &'static str {
    if !ko {
        return ee_core::report::path_str(p);
    }
    match p {
        Path::Capacitive => "용량성",
        Path::Inductive => "유도성",
        Path::ModeConversion => "모드 변환",
        Path::PlaneGap => "평면 틈",
        Path::HotLoop => "핫 루프",
        Path::Cable => "보드 간 케이블",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_html_key_has_both_languages() {
        let html = include_str!("../../../index.html");
        let keys: Vec<&str> = html.split("data-t=\"").skip(1).filter_map(|s| s.split('"').next()).collect();
        assert!(keys.len() > 30, "{}", keys.len());
        for k in keys.iter().chain(DYNAMIC) {
            let e = T.iter().find(|e| e.0 == *k).unwrap_or_else(|| panic!("missing key {k}"));
            assert!(!e.1.is_empty() && !e.2.is_empty(), "{k}");
        }
        assert_eq!(tr(true, "h_load"), "파일 불러오기");
        assert_eq!(tr(false, "h_load"), "Load a board");
        assert_eq!(tr(true, "no_such_key"), "no_such_key");
    }

    #[test]
    fn fix_and_path_text_in_both_languages() {
        use ee_core::couple::{FixKind, Path};
        let f = FixKind::MovePair { mm: 2.5 };
        assert_eq!(fix_text(false, &f, "RX"), ee_core::report::fix_str(&f, "RX"));
        assert!(fix_text(true, &f, "RX").contains("2.5 mm"));
        assert_eq!(path_text(false, Path::PlaneGap), "plane gap");
        assert_ne!(path_text(true, Path::PlaneGap), "plane gap");
    }
}
