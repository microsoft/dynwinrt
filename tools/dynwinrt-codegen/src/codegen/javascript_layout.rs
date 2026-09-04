// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

pub(crate) fn canonical_namespace_path(namespace: &str) -> String {
    namespace
        .split('.')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let mut result = String::new();
            let chars = segment.chars().collect::<Vec<_>>();
            for (index, character) in chars.iter().copied().enumerate() {
                if !character.is_ascii_alphanumeric() {
                    if !result.ends_with('-') {
                        result.push('-');
                    }
                    continue;
                }
                let previous_is_lower_or_digit = index > 0
                    && (chars[index - 1].is_ascii_lowercase() || chars[index - 1].is_ascii_digit());
                let next_is_lower = chars
                    .get(index + 1)
                    .is_some_and(|next| next.is_ascii_lowercase());
                if character.is_ascii_uppercase()
                    && !result.is_empty()
                    && (previous_is_lower_or_digit || next_is_lower)
                    && !result.ends_with('-')
                {
                    result.push('-');
                }
                result.push(character.to_ascii_lowercase());
            }
            result.trim_matches('-').to_string()
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespaces_use_lowercase_kebab_segments() {
        assert_eq!(
            canonical_namespace_path("Microsoft.UI.DragDrop"),
            "microsoft/ui/drag-drop"
        );
        assert_eq!(
            canonical_namespace_path("Windows.Win32.Graphics.Direct2D"),
            "windows/win32/graphics/direct2-d"
        );
    }
}
