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
    /// SelectedTags="a b c">`, space-separated). Collected up front so a channel
    /// can inherit its ancestor groups' tags regardless of document order
    /// (#170). NOTE: `SelectedTags` is the attribute name documented in the
    /// issue; it is absent from both verification corpora, so this is
    /// spec-grounded but unverified against real data — an absent attribute is a
    /// pure no-op, and a future schema correction is a one-line change here.
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
                let tags = n
                    .children()
                    .find(|c| c.has_tag_name("Props"))?
                    .attribute("SelectedTags")?
                    .split_whitespace()
                    .map(str::to_string)
                    .collect::<Vec<_>>();
                (!tags.is_empty()).then(|| (name.to_string(), tags))
            })
            .collect();

        ProjectXmlIndex {
            classname_by_path,
            selected_tags_by_path,
        }
    }
}
