//! Up-front pre-pass indexes over the parsed `.m1prj` `roxmltree::Document`.
//!
//! These maps are built in a single descendant walk before the main component
//! pass so that a component can be typed/tagged regardless of document order
//! (a channel may reference its owning object's class, or inherit an ancestor
//! group's tags, before that ancestor appears in the file).

use std::collections::HashMap;

/// Pre-pass indexes built from the project XML, read by the component pass.
pub(super) struct ProjectXmlIndex {
    /// Every component's path -> its `Classname`, so a channel that carries no
    /// inline `<Props Type>`/`Qty` can be typed from the class of the object
    /// that owns it (its parent). M1 Build derives these the same way — the
    /// object's class is the type source (#25).
    pub(super) classname_by_path: HashMap<String, String>,
    /// Every component's path -> the tags it declares directly (`<Props
    /// SelectedTags="a b c">` or `<List.UserTags><Entry Value="…">`). Collected up front so a channel
    /// can inherit its ancestor groups' tags regardless of document order
    /// (#170). Real M1-Build projects use `List.UserTags`; `SelectedTags` remains
    /// accepted for older fixtures and hand-authored inputs.
    pub(super) selected_tags_by_path: HashMap<String, Vec<String>>,
}

impl ProjectXmlIndex {
    /// Build both pre-pass indexes in a pair of descendant walks over the
    /// project document.
    pub(super) fn build(doc: &roxmltree::Document) -> Self {
        let classname_by_path: HashMap<String, String> = doc
            .descendants()
            .filter(|n| n.has_tag_name("Component"))
            .filter_map(|n| {
                Some((
                    n.attribute("Name")?.to_string(),
                    n.attribute("Classname")?.to_string(),
                ))
            })
            .collect();

        let selected_tags_by_path: HashMap<String, Vec<String>> = doc
            .descendants()
            .filter(|n| n.has_tag_name("Component"))
            .filter_map(|n| {
                let name = n.attribute("Name")?;
                let props = n.children().find(|c| c.has_tag_name("Props"))?;
                let mut tags = props
                    .attribute("SelectedTags")
                    .into_iter()
                    .flat_map(str::split_whitespace)
                    .map(str::to_string)
                    .collect::<Vec<_>>();
                if let Some(list) = props.children().find(|c| c.has_tag_name("List.UserTags")) {
                    for tag in list
                        .children()
                        .filter(|entry| entry.has_tag_name("Entry"))
                        .filter_map(|entry| entry.attribute("Value"))
                    {
                        if !tags
                            .iter()
                            .any(|existing| existing.eq_ignore_ascii_case(tag))
                        {
                            tags.push(tag.to_string());
                        }
                    }
                }
                (!tags.is_empty()).then(|| (name.to_string(), tags))
            })
            .collect();

        ProjectXmlIndex {
            classname_by_path,
            selected_tags_by_path,
        }
    }
}
