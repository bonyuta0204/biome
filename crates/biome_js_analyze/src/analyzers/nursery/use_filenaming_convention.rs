use std::{hash::Hash, str::FromStr};

use crate::{semantic_services::SemanticServices, utils::case::Case};
use biome_analyze::{context::RuleContext, declare_rule, Rule, RuleDiagnostic};
use biome_console::markup;
use biome_deserialize::{
    Deserializable, DeserializationDiagnostic, DeserializationVisitor, Text, VisitableType,
};
use biome_rowan::TextRange;
use rustc_hash::FxHashSet;
use serde::{Deserialize, Serialize};

#[cfg(feature = "schemars")]
use schemars::JsonSchema;
use smallvec::SmallVec;

declare_rule! {
    /// Enforce naming conventions for JavaScript and TypeScript filenames.
    ///
    /// Enforcing [naming conventions](https://en.wikipedia.org/wiki/Naming_convention_(programming)) helps to keep the codebase consistent.
    ///
    /// A filename consists of two parts: a name and a set of consecutive extension.
    /// For instance, `my-filename.test.js` has `my-filename` as name, and two consecutive extensions: `.test` and `.js`.
    ///
    /// The name of a filename can start with a dot, be prefixed and suffixed by underscores `_`.
    /// For example, `.filename.js`, `__filename__.js`, or even `.__filename__.js`.
    ///
    /// By default, the rule ensures that the filename is either in [`camelCase`], [`kebab-case`], [`snake_case`],
    /// or equal to the name of one export in the file.
    ///
    /// ## Options
    ///
    /// The rule provides two options that are detailed in the following subsections.
    ///
    /// ```json
    /// {
    ///     "//": "...",
    ///     "options": {
    ///         "strictCase": false,
    ///         "enumMemberCase": ["camelCase", "export"]
    ///     }
    /// }
    /// ```
    ///
    /// ### strictCase
    ///
    /// When this option is set to `true`, it forbids consecutive uppercase characters in [`camelCase`].
    /// For instance,  when the option is set to `true`, `agentID` will throw an error.
    /// This name should be renamed to `agentId`.
    ///
    /// When the option is set to `false`, consecutive uppercase characters are allowed.
    /// `agentID` is so valid.
    ///
    /// Default: `true`
    ///
    /// ### filenameCases
    ///
    /// By default, the rule enforces that the filename  is either in [`camelCase`], [`kebab-case`], [`snake_case`], or equal to the name of one export in the file.
    ///
    /// You can enforce a stricter convention by setting `filenameCases` option.
    /// `filenameCases` accepts an array of cases among the following cases: [`camelCase`], [`kebab-case`], [`PascalCase`], [`snake_case`], and `export`.
    ///
    /// [case]: https://en.wikipedia.org/wiki/Naming_convention_(programming)#Examples_of_multiple-word_identifier_formats
    /// [`camelCase`]: https://en.wikipedia.org/wiki/Camel_case
    /// [`kebab-case`]: https://en.wikipedia.org/wiki/Letter_case#Kebab_case
    /// [`PascalCase`]: https://en.wikipedia.org/wiki/Camel_case
    /// [`snake_case`]: https://en.wikipedia.org/wiki/Snake_case
    pub(crate) UseFilenamingConvention {
        version: "next",
        name: "useFilenamingConvention",
        recommended: false,
    }
}

impl Rule for UseFilenamingConvention {
    type Query = SemanticServices;
    type State = FileNamingConventionState;
    type Signals = Option<Self::State>;
    type Options = FilenamingConventionOptions;

    fn run(ctx: &RuleContext<Self>) -> Self::Signals {
        let file_name = ctx.file_path().file_name()?.to_str()?;
        let FilenamingConventionOptions {
            strict_case,
            filename_cases,
        } = ctx.options();
        let allowed_cases = filename_cases.cases();
        let mut splitted = file_name.split('.');
        let name = splitted.next()?;
        let name = if name.is_empty() {
            // The filename starts with a dot
            splitted.next()?
        } else {
            name
        };
        // Check extension case
        for extension in splitted {
            let case = Case::identify(extension, true);
            if case != Case::Lower {
                return Some(FileNamingConventionState::Extension);
            }
        }
        // Check filename case
        if !allowed_cases.is_empty() {
            let trimmed_name = name.trim_matches('_');
            let case = Case::identify(trimmed_name, *strict_case);
            for allowed_case in allowed_cases {
                if case.is_compatible_with(allowed_case) {
                    return None;
                }
            }
        }
        if filename_cases.0.contains(&FilenameCase::Export) {
            // If no exported binding has the file name, then reports the filename
            let model = ctx.model();
            model
                .all_bindings()
                .map(|binding| binding.tree())
                .filter(|binding| model.is_exported(binding))
                .filter_map(|exported_binding| exported_binding.name_token().ok())
                .all(|exported_name_token| exported_name_token.text_trimmed() != name)
                .then_some(FileNamingConventionState::Filename)
        } else {
            Some(FileNamingConventionState::Filename)
        }
    }

    fn diagnostic(ctx: &RuleContext<Self>, state: &Self::State) -> Option<RuleDiagnostic> {
        let file_name = ctx.file_path().file_name()?.to_str()?;
        let FilenamingConventionOptions {
            strict_case,
            filename_cases,
        } = ctx.options();
        if matches!(state, FileNamingConventionState::Filename) {
            let allowed_cases = filename_cases.cases();
            let allowed_case_names = allowed_cases.iter().map(|style| style.to_string());
            let allowed_case_names = if filename_cases.0.contains(&FilenameCase::Export) {
                allowed_case_names
                    .chain(["equal to the name of an export".to_string()])
                    .collect::<SmallVec<[_; 4]>>()
                    .join(" or ")
            } else {
                allowed_case_names
                    .collect::<SmallVec<[_; 3]>>()
                    .join(" or ")
            };
            let mut splitted = file_name.split('.');
            let name = splitted.next()?;
            let name = if name.is_empty() {
                // The filename starts with a dot
                splitted.next()?
            } else {
                name
            };
            let trimmed_name = name.trim_matches('_');
            let trimmed_info = if name != trimmed_name {
                markup! {" trimmed as `"{trimmed_name}"`"}.to_owned()
            } else {
                markup! {""}.to_owned()
            };
            if *strict_case && filename_cases.0.contains(&FilenameCase::Camel) {
                let case_type = Case::identify(trimmed_name, false);
                let case_strict = Case::identify(trimmed_name, true);
                if case_type == Case::Camel && case_strict == Case::Unknown {
                    return Some(RuleDiagnostic::new(
                        rule_category!(),
                        None as Option<TextRange>,
                        markup! {
                            "The filename"{trimmed_info}" is in camelCase, however, two consecutive uppercase characters are not allowed because `strictCase` is set to `true`."
                        },
                    ).note(markup! {
                        "If you want to use consecutive uppercase characters in camelCase then consider setting `strictCase` option to `false`.\n Check rule "<Hyperlink href="https://biomejs.dev/linter/rules/use-filenaming-convention#options">"options"</Hyperlink>" for more information."
                    }));
                }
            }
            let suggested_filenames = allowed_cases
                .iter()
                .map(|case| file_name.replacen(trimmed_name, &case.convert(trimmed_name), 1))
                // Deduplicate suggestions
                .collect::<FxHashSet<_>>()
                .into_iter()
                .collect::<SmallVec<[_; 3]>>()
                .join("\n");
            Some(RuleDiagnostic::new(
                rule_category!(),
                None as Option<TextRange>,
                markup! {
                    "The filename"{trimmed_info}" should be in "<Emphasis>{allowed_case_names}</Emphasis>"."
                },
            ).note(markup! {
                "The filename could be renamed to one of the following names:\n"{suggested_filenames}
            }))
        } else {
            Some(RuleDiagnostic::new(
                rule_category!(),
                None as Option<TextRange>,
                markup! {
                    "The file extension should be in lowercase without any special characters."
                },
            ))
        }
    }
}

#[derive(Debug)]
pub(crate) enum FileNamingConventionState {
    Filename,
    Extension,
}

/// Rule's options.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FilenamingConventionOptions {
    /// If `false`, then consecutive uppercase are allowed in _camel_ and _pascal_ cases.
    /// This does not affect other [Case].
    #[serde(
        default = "default_strict_case",
        skip_serializing_if = "is_default_strict_case"
    )]
    pub strict_case: bool,

    /// Allowed cases for _TypeScript_ `enum` member names.
    #[serde(default, skip_serializing_if = "is_default_filename_cases")]
    pub filename_cases: FilenameCases,
}

const fn default_strict_case() -> bool {
    true
}

const fn is_default_strict_case(strict_case: &bool) -> bool {
    *strict_case == default_strict_case()
}

fn is_default_filename_cases(value: &FilenameCases) -> bool {
    value.0.len() == 4 && !value.0.contains(&FilenameCase::Pascal)
}

impl Default for FilenamingConventionOptions {
    fn default() -> Self {
        Self {
            strict_case: default_strict_case(),
            filename_cases: FilenameCases::default(),
        }
    }
}

impl Deserializable for FilenamingConventionOptions {
    fn deserialize(
        value: &impl biome_deserialize::DeserializableValue,
        name: &str,
        diagnostics: &mut Vec<DeserializationDiagnostic>,
    ) -> Option<Self> {
        value.deserialize(FilenamingConventionOptionsVisitor, name, diagnostics)
    }
}

struct FilenamingConventionOptionsVisitor;
impl DeserializationVisitor for FilenamingConventionOptionsVisitor {
    type Output = FilenamingConventionOptions;

    const EXPECTED_TYPE: VisitableType = VisitableType::MAP;

    fn visit_map(
        self,
        members: impl Iterator<
            Item = Option<(
                impl biome_deserialize::DeserializableValue,
                impl biome_deserialize::DeserializableValue,
            )>,
        >,
        _range: biome_rowan::TextRange,
        _name: &str,
        diagnostics: &mut Vec<DeserializationDiagnostic>,
    ) -> Option<Self::Output> {
        const ALLOWED_KEYS: &[&str] = &["strictCase", "filenameCases"];
        let mut result = Self::Output::default();
        for (key, value) in members.flatten() {
            let Some(key_text) = Text::deserialize(&key, "", diagnostics) else {
                continue;
            };
            match key_text.text() {
                "strictCase" => {
                    if let Some(strict_case) =
                        Deserializable::deserialize(&value, &key_text, diagnostics)
                    {
                        result.strict_case = strict_case;
                    }
                }
                "filenameCases" => {
                    if let Some(filename_cases) =
                        Deserializable::deserialize(&value, &key_text, diagnostics)
                    {
                        result.filename_cases = filename_cases;
                    }
                }
                unknown_key => diagnostics.push(DeserializationDiagnostic::new_unknown_key(
                    unknown_key,
                    key.range(),
                    ALLOWED_KEYS,
                )),
            }
        }
        Some(result)
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(JsonSchema))]
pub struct FilenameCases(FxHashSet<FilenameCase>);

impl FilenameCases {
    fn cases(&self) -> SmallVec<[Case; 3]> {
        self.0
            .iter()
            .filter_map(|case| Case::try_from(*case).ok())
            .collect()
    }
}

impl Default for FilenameCases {
    fn default() -> Self {
        Self(FxHashSet::from_iter([
            FilenameCase::Camel,
            FilenameCase::Export,
            FilenameCase::Kebab,
            FilenameCase::Snake,
        ]))
    }
}

impl Deserializable for FilenameCases {
    fn deserialize(
        value: &impl biome_deserialize::DeserializableValue,
        name: &str,
        diagnostics: &mut Vec<DeserializationDiagnostic>,
    ) -> Option<Self> {
        let cases: FxHashSet<_> = Deserializable::deserialize(value, name, diagnostics)?;
        if cases.is_empty() {
            diagnostics.push(
                DeserializationDiagnostic::new(markup! {
                    ""<Emphasis>{name}</Emphasis>" cannot be an empty array."
                })
                .with_range(value.range()),
            );
            return None;
        }
        Some(Self(cases))
    }
}

/// Supported cases for TypeScript `enum` member names.
#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(JsonSchema))]
pub enum FilenameCase {
    /// camelCase
    #[serde(rename = "camelCase")]
    Camel,

    /// Match an export name
    #[serde(rename = "export")]
    Export,

    /// kebab-case
    #[serde(rename = "kebab-case")]
    Kebab,

    /// PascalCase
    #[serde(rename = "PascalCase")]
    Pascal,

    /// snake_case
    #[serde(rename = "snake_case")]
    Snake,
}

impl FilenameCase {
    pub const ALLOWED_VARIANTS: &'static [&'static str] = &[
        "camelCase",
        "export",
        "kebab-case",
        "PascalCase",
        "snake_case",
    ];
}

impl FromStr for FilenameCase {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "camelCase" => Ok(Self::Camel),
            "export" => Ok(Self::Export),
            "kebab-case" => Ok(Self::Kebab),
            "PascalCase" => Ok(Self::Pascal),
            "snake_case" => Ok(Self::Snake),
            _ => Err("Value not supported for enum member case"),
        }
    }
}

impl Deserializable for FilenameCase {
    fn deserialize(
        value: &impl biome_deserialize::DeserializableValue,
        name: &str,
        diagnostics: &mut Vec<DeserializationDiagnostic>,
    ) -> Option<Self> {
        let value_text = Text::deserialize(value, name, diagnostics)?;
        if let Ok(value) = value_text.parse::<Self>() {
            Some(value)
        } else {
            diagnostics.push(DeserializationDiagnostic::new_unknown_value(
                value_text.text(),
                value.range(),
                Self::ALLOWED_VARIANTS,
            ));
            None
        }
    }
}

impl TryFrom<FilenameCase> for Case {
    type Error = &'static str;

    fn try_from(case: FilenameCase) -> Result<Self, Self::Error> {
        match case {
            FilenameCase::Camel => Ok(Self::Camel),
            FilenameCase::Export => Err("`export` is not a valid case"),
            FilenameCase::Kebab => Ok(Self::Kebab),
            FilenameCase::Pascal => Ok(Self::Pascal),
            FilenameCase::Snake => Ok(Self::Snake),
        }
    }
}
