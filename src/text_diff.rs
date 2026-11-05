//! Standalone text comparison. No repository, temporary files or Git process.
use gpui_kit::component::diff::DiffFile;
use similar::TextDiff;
use std::{path::Path, time::Duration};

pub fn compare(left: &str, right: &str) -> anyhow::Result<DiffFile> {
    if left == right {
        return Ok(DiffFile::unchanged("comparison", left));
    }
    // A deadline bounds the expensive search for an optimal alignment; even
    // when it expires, similar still emits a complete, valid transformation.
    let diff = TextDiff::configure()
        .timeout(Duration::from_secs(2))
        .diff_lines(left, right);
    let context = diff.old_slices().len().max(diff.new_slices().len());
    let patch = diff
        .unified_diff()
        .context_radius(context)
        .header("a/comparison", "b/comparison")
        .to_string();
    DiffFile::parse(&patch)?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("The comparison produced no document"))
}

#[derive(Debug)]
pub enum ReadTextError {
    Io(std::io::Error),
    NotText,
}

pub fn read_text(path: &Path) -> Result<String, ReadTextError> {
    let bytes = std::fs::read(path).map_err(ReadTextError::Io)?;
    if bytes.contains(&0) {
        return Err(ReadTextError::NotText);
    }
    String::from_utf8(bytes).map_err(|_| ReadTextError::NotText)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::component::diff::{DiffLineRange, DiffSide, DiffState};
    use gpui_kit::{AppContext, TestAppContext};

    #[gpui_kit::test]
    fn comparison_preserves_both_sides_and_context(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        for (left, right) in [
            ("", ""),
            ("same\ntext\n", "same\ntext\n"),
            ("", "新增\n🙂"),
            ("removed\n", ""),
            ("a\r\nb\r\n", "a\nb\n"),
            ("last\n", "last"),
            ("same\nold\nsame\n", "same\nnew\nsame\n"),
            ("--- a\n@@\n\\", "+++ b\n@@\n\\\n"),
        ] {
            let file = compare(left, right).unwrap();
            let state = cx.new(|cx| DiffState::new([file.clone()], cx));
            for (side, text) in [(DiffSide::Original, left), (DiffSide::Modified, right)] {
                // Identical documents use the renderer's single reference column.
                if left == right && side == DiffSide::Original {
                    continue;
                }
                let range =
                    DiffLineRange::new(file.path().clone(), side, 1, text.lines().count().max(1));
                state.update(cx, |state, cx| {
                    state.set_selected_lines(Some(range), cx);
                    assert_eq!(state.selected_text(cx), text, "{left:?} -> {right:?}");
                });
            }
        }
    }

    #[test]
    fn reads_snapshots_without_modifying_source_and_rejects_binary() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("中文 [draft].txt");
        let text = "one\r\n二\nlast";
        std::fs::write(&path, text).unwrap();
        let mut snapshot = read_text(&path).unwrap();
        snapshot.push_str(" edited");
        compare(text, &snapshot).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        for bytes in [vec![0, 1, 2], vec![255, 254]] {
            std::fs::write(&path, bytes).unwrap();
            assert!(matches!(read_text(&path), Err(ReadTextError::NotText)));
        }
    }
}
