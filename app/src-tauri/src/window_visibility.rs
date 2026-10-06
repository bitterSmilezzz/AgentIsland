//! Visible metadata corroborates a retained AX object; titles never participate.
use crate::window_layout::Rect;
pub fn match_id(
    pid: i32,
    rect: Rect,
    ax_member: bool,
    same_ax_geometry: usize,
    visible: &[(i32, u32, Rect)],
) -> Result<u32, String> {
    if !ax_member {
        return Err("窗口已关闭或不属于当前工具".into());
    }
    if same_ax_geometry != 1 {
        return Err("重叠窗口无法唯一核实，请先错开窗口".into());
    }
    let mut ids = visible
        .iter()
        .filter(|(owner, _, bounds)| {
            *owner == pid && crate::window_layout_execution::matches(*bounds, rect)
        })
        .map(|(_, id, _)| *id);
    let first = ids.next().ok_or("窗口不在当前桌面或可见性无法核实")?;
    if ids.next().is_some() {
        return Err("窗口可见性无法唯一核实".into());
    }
    Ok(first)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn rect() -> Rect {
        Rect {
            x: -1200.0,
            y: 24.0,
            width: 700.0,
            height: 800.0,
        }
    }
    #[test]
    fn visible_match_requires_owner_member_and_unique_ax_geometry() {
        let visible = vec![(10, 7, rect())];
        assert_eq!(match_id(10, rect(), true, 1, &visible).unwrap(), 7);
        assert!(match_id(11, rect(), true, 1, &visible).is_err());
        assert!(match_id(10, rect(), false, 1, &visible).is_err());
        assert!(match_id(10, rect(), true, 2, &visible).is_err());
    }
    #[test]
    fn hidden_and_ambiguous_visible_windows_do_not_get_an_identity() {
        assert!(match_id(10, rect(), true, 1, &[]).is_err());
        assert!(match_id(10, rect(), true, 1, &[(10, 7, rect()), (10, 8, rect())]).is_err());
    }
}
