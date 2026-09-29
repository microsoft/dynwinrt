## Unreleased (next preview)

Draft notes for a future release, not part of v0.1.0-preview.22. Before tagging
the next release, promote the applicable entries to `RELEASE_NOTES.md` under
the correct version heading. The release pipeline does not stage this file.

### Python Object values

- Explicitly box Python values with `to_winrt_object()` and convert values in
  supported Object-valued maps with `dynwinrt.values.object_value_view()`.
  Generated `Object` positions remain native by default.

### Rust source compatibility

- Exhaustive matches on the public `PropertyValueData` and
  `PropertyValueUnboxResult` enums must add arms for the new payload variants
  and `PropertyValueUnboxResult::Unsupported(PropertyType)`. Handle unsupported
  boxes explicitly; keep `Null` and `NotPropertyValue` distinct.
