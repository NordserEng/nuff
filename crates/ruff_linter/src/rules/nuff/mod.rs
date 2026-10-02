//! Nordser's house rules.
pub(crate) mod rules;
pub mod settings;

#[cfg(test)]
mod tests {
    use std::path::Path;

    use anyhow::Result;
    use test_case::test_case;

    use crate::registry::Rule;
    use crate::settings::LinterSettings;
    use crate::test::test_path;
    use crate::{assert_diagnostics, rules::nuff};

    fn settings(rule: Rule) -> LinterSettings {
        LinterSettings {
            nuff: nuff::settings::Settings {
                blocking_functions: vec![
                    "app.platform.email.send_email".to_string(),
                    "app.accounts.services.brreg.fetch_company".to_string(),
                    "xml.etree.ElementTree.fromstring".to_string(),
                ],
                external_functions: vec![
                    "app.accounts.services.brreg.fetch_company".to_string(),
                    "app.billing.services.vipps.agreement_status".to_string(),
                    "app.billing.services.vipps.create_charge".to_string(),
                    "app.billing.services.vipps.list_charges".to_string(),
                    "app.billing.services.vipps.stop_agreement".to_string(),
                ],
                foreign_key_modules: [
                    ("companies", "app.accounts.models"),
                    ("charges", "app.billing.models"),
                    ("receipts", "app.billing.models"),
                ]
                .into_iter()
                .map(|(table, module)| (table.to_string(), module.to_string()))
                .collect(),
            },
            ..LinterSettings::for_rule(rule)
        }
    }

    #[test_case(Rule::BlockingCallOutsideThread, Path::new("NUF001.py"))]
    #[test_case(Rule::RouteReturnsDict, Path::new("NUF002.py"))]
    #[test_case(Rule::AlembicDowngrade, Path::new("NUF003.py"))]
    #[test_case(Rule::AlembicDowngrade, Path::new("NUF003_upgrade_only.py"))]
    #[test_case(Rule::AlembicDowngrade, Path::new("NUF003_not_a_revision.py"))]
    #[test_case(Rule::ForeignKeyModelNotImported, Path::new("app/billing/models.py"))]
    #[test_case(Rule::ForeignKeyModelNotImported, Path::new("app/books/models.py"))]
    #[test_case(Rule::ForeignKeyModelNotImported, Path::new("NUF004_migration.py"))]
    #[test_case(Rule::ExternalCallInTransaction, Path::new("NUF005.py"))]
    fn rules(rule_code: Rule, path: &Path) -> Result<()> {
        let snapshot = format!(
            "{}_{}",
            rule_code.name(),
            path.to_string_lossy().replace('/', "_")
        );
        let diagnostics = test_path(Path::new("nuff").join(path).as_path(), &settings(rule_code))?;
        assert_diagnostics!(snapshot, diagnostics);
        Ok(())
    }

    #[test_case(Rule::BlockingCallOutsideThread, Path::new("NUF001.py"))]
    #[test_case(Rule::ForeignKeyModelNotImported, Path::new("app/billing/models.py"))]
    #[test_case(Rule::ExternalCallInTransaction, Path::new("NUF005.py"))]
    fn unconfigured(rule_code: Rule, path: &Path) -> Result<()> {
        let snapshot = format!(
            "unconfigured_{}_{}",
            rule_code.name(),
            path.to_string_lossy().replace('/', "_")
        );
        let diagnostics = test_path(
            Path::new("nuff").join(path).as_path(),
            &LinterSettings::for_rule(rule_code),
        )?;
        assert_diagnostics!(snapshot, diagnostics);
        Ok(())
    }
}
