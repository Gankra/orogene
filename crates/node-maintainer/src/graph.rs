use std::{
    collections::VecDeque,
    ops::{Index, IndexMut},
};

use kdl::{KdlDocument, KdlNode};
use nassun::PackageResolution;
use petgraph::{
    dot::Dot,
    stable_graph::{NodeIndex, StableGraph},
};
use unicase::UniCase;

use crate::{Edge, Node, NodeMaintainerError};

#[derive(Debug, Default)]
pub struct Graph {
    pub(crate) root: NodeIndex,
    pub(crate) inner: StableGraph<Node, Edge>,
}

impl Index<NodeIndex> for Graph {
    type Output = Node;

    fn index(&self, index: NodeIndex) -> &Self::Output {
        &self.inner[index]
    }
}

impl IndexMut<NodeIndex> for Graph {
    fn index_mut(&mut self, index: NodeIndex) -> &mut Self::Output {
        &mut self.inner[index]
    }
}

impl Graph {
    fn node_kdl(&self, node: NodeIndex, is_root: bool) -> KdlNode {
        let node = &self.inner[node];
        let mut pathnames = VecDeque::new();
        pathnames.push_front(node.package.name());
        let mut kdl_node = if is_root {
            KdlNode::new("root")
        } else {
            let mut parent = node.parent;
            while let Some(parent_idx) = parent {
                if parent_idx == self.root {
                    break;
                }
                pathnames.push_front(self.inner[parent_idx].package.name());
                parent = self.inner[parent_idx].parent;
            }
            KdlNode::new("dep")
        };
        for name in pathnames.drain(..) {
            kdl_node.push(name);
        }
        let resolved = node.package.resolved();
        if let &PackageResolution::Npm { version, .. } = &resolved {
            let mut vnode = KdlNode::new("version");
            vnode.push(version.to_string());
            kdl_node.ensure_children().nodes_mut().push(vnode);
        }
        if !is_root {
            let mut rnode = KdlNode::new("resolved");
            rnode.push(resolved.to_string());
            kdl_node.ensure_children().nodes_mut().push(rnode);

            if let &PackageResolution::Npm {
                integrity: Some(i), ..
            } = &resolved
            {
                let mut inode = KdlNode::new("integrity");
                inode.push(i.to_string());
            }
        }
        if !node.dependencies.is_empty() {
            let mut dependencies = node
                .dependencies
                .iter()
                .map(|(name, edge_idx)| {
                    let edge = &self.inner[*edge_idx];
                    (name, &edge.requested, &edge.dep_type)
                })
                .collect::<Vec<_>>();
            dependencies.sort_by(
                |(n1, _, dt1), (n2, _, dt2)| {
                    if dt1 == dt2 {
                        n1.cmp(n2)
                    } else {
                        dt1.cmp(dt2)
                    }
                },
            );
            for (name, requested, dep_type) in dependencies {
                use crate::DepType::*;
                let type_name = match dep_type {
                    Prod => "dependencies",
                    Dev => "devDependencies",
                    Peer => "peerDependencies",
                    Opt => "optionalDependencies",
                };
                let dnode = if let Some(dnode) = kdl_node.ensure_children().get_mut(type_name) {
                    dnode
                } else {
                    kdl_node
                        .ensure_children()
                        .nodes_mut()
                        .push(KdlNode::new(type_name));
                    kdl_node.ensure_children().get_mut(type_name).unwrap()
                };
                let children = dnode.ensure_children();
                let mut ddnode = KdlNode::new(name.to_string());
                ddnode.push(requested.requested());
                children.nodes_mut().push(ddnode);
            }
        }
        kdl_node
    }

    pub fn to_kdl(&self) -> KdlDocument {
        let mut doc = KdlDocument::new();
        doc.nodes_mut().push("lockfile-version 1".parse().unwrap());
        doc.nodes_mut().push(KdlNode::new("packages"));
        let packages_node = doc.get_mut("packages").unwrap();
        packages_node
            .ensure_children()
            .nodes_mut()
            .push(self.node_kdl(self.root, true));
        let mut other_nodes = self
            .inner
            .node_indices()
            .filter(|idx| *idx != self.root)
            .map(|node| self.node_kdl(node, false))
            .collect::<Vec<_>>();
        other_nodes.sort_by_key(|n1| n1.name().to_string());
        packages_node
            .ensure_children()
            .nodes_mut()
            .append(&mut other_nodes);
        doc.fmt();
        doc
    }

    pub fn render(&self) -> String {
        format!(
            "{:?}",
            Dot::new(&self.inner.map(
                |_, mut node| {
                    let resolved = node.package.resolved();
                    let mut label = node.package.name().to_string();
                    while let Some(node_idx) = &node.parent {
                        node = &self.inner[*node_idx];
                        let name = node.package.name();
                        label = format!("{name}/node_modules/{label}");
                    }
                    format!("{resolved:?} @ {label}")
                },
                |_, edge| { format!("{}", edge.requested) }
            ))
        )
    }

    pub(crate) fn find_by_name(
        &self,
        parent: NodeIndex,
        name: &UniCase<String>,
    ) -> Result<Option<NodeIndex>, NodeMaintainerError> {
        let mut parent = self.inner.node_weight(parent);
        while let Some(node) = parent {
            if node.children.contains_key(name) {
                return Ok(Some(node.children[name]));
            }
            parent = node.parent.and_then(|idx| self.inner.node_weight(idx));
        }
        Ok(None)
    }
}