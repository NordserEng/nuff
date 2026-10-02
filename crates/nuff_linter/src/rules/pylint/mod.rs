//! Rules from [Pylint](https://pypi.org/project/pylint/).
pub(crate) mod rules;

#[cfg(test)]
mod tests {
    use std::path::Path;

    use anyhow::Result;

    use crate::assert_diagnostics;
    use crate::registry::Rule;
    use crate::settings::LinterSettings;
    use crate::test::test_path;

    #[test]
    fn import_outside_top_level() -> Result<()> {
        let diagnostics = test_path(
            Path::new("pylint/import_outside_top_level.py"),
            &LinterSettings::for_rule(Rule::ImportOutsideTopLevel),
        )?;
        assert_diagnostics!(
            "import-outside-top-level_import_outside_top_level.py",
            diagnostics
        );
        Ok(())
    }
}
