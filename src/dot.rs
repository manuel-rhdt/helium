use crate::tree::{TreeLinked, OpaqueNode, GetOpaqueNode};

use std::io::{Write, Result};
use std::fmt;
use std::collections::HashMap;

#[derive(Debug, Copy, Clone)]
pub enum Shape {
    Box,
    Circle,
    Ellipse,
    Plaintext
}

#[derive(Debug, Copy, Clone)]
pub enum Style {
    /// fill color
    Filled(f32, f32, f32)
}

impl fmt::Display for Style {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Style::Filled(r, g, b) => write!(f, "style=filled,color=\"{} {} {}\"", r, g, b),
        }
    }
}

impl Default for Shape {
    fn default() -> Shape {
        Shape::Ellipse
    }
}

impl fmt::Display for Shape {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Shape::Box => write!(f, "shape=box"),
            Shape::Circle => write!(f, "shape=circle"),
            Shape::Ellipse => write!(f, "shape=ellipse"),
            Shape::Plaintext => write!(f, "shape=plaintext")
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct GraphDisplayStyle {
    pub label: String,
    pub shape: Shape,
    pub style: Option<Style>
}

impl GraphDisplayStyle {
    pub fn new() -> GraphDisplayStyle {
        Default::default()
    }

    pub fn label(self, label: impl std::fmt::Display) -> Self {
        GraphDisplayStyle {
            label: label.to_string(),
            ..self
        }
    }

    pub fn shape(self, shape: Shape) -> Self {
        GraphDisplayStyle {
            shape,
            ..self
        }
    }

    pub fn style(self, style: Style) -> Self {
        GraphDisplayStyle {
            style: Some(style),
            ..self
        }
    }

    pub fn write(&self, mut w: impl Write) -> Result<()> {
        write!(w, "[{},label=\"{}\"", self.shape, self.label)?;
        if let Some(style) = self.style {
            write!(w, ",{}", style)?;
        }
        write!(w, "]")
    }
}

#[derive(Debug, Copy, Clone)]
pub enum Connection {
    Root,
    DescendantOf(OpaqueNode)
}

pub struct DotGraphSerializer<Output> {
    output: Output,
    styles: Vec<GraphDisplayStyle>,
    node_index_map: HashMap<OpaqueNode, usize>
}

impl<Output: Write> DotGraphSerializer<Output> {
    pub fn new(output: Output) -> Self {
        DotGraphSerializer {
            output,
            styles: vec![],
            node_index_map: HashMap::new()
        }
    }

    pub fn begin_graph(&mut self, name: impl fmt::Display) -> Result<()> {
        writeln!(self.output, "digraph {} {{", name)
    }

    fn write_edge(&mut self, a: usize, b: usize) -> Result<()> {
        writeln!(self.output, "\tnode{} -> node{};", a, b)?;
        Ok(())
    }

    pub fn contains_node(&self, node: OpaqueNode) -> bool {
        self.node_index_map.contains_key(&node)
    }

    /// # Panics
    /// Panics if the connection contains a node that was not pushed before.
    pub fn push_node<Node>(&mut self, node: Node, connection: Connection) -> Result<()> where Node: GraphStyle {
        let opaque = node.get_opaque();
        let node_index = if let Some(&index) = self.node_index_map.get(&opaque) {
            index
        } else {
            self.styles.push(node.graph_style());
            let index = self.styles.len() - 1;
            self.node_index_map.insert(opaque, index);
            index
        };

        if let Connection::DescendantOf(parent) = &connection {
            self.write_edge(self.node_index_map[parent], node_index)?
        }

        Ok(())
    }

    pub fn end_graph(&mut self) -> Result<()> {
        for (index, node) in self.styles.iter().enumerate() {
            write!(self.output, "\tnode{} ", index)?;
            node.write(&mut self.output)?;
            writeln!(self.output, ";")?;
        }

        writeln!(self.output, "}}")?;
        Ok(())
    }
    
    fn write_dot<Node>(&mut self, root_node: Node) -> Result<()> where Node: GraphStyle + TreeLinked {
        self.begin_graph("G")?;
        self.push_node(root_node.clone(), Connection::Root)?;
        self.write_all_children(root_node)?;
        self.end_graph()
    }

    fn write_all_children<Node>(&mut self, node: Node) -> Result<()> where Node: GraphStyle + TreeLinked {
        for child in node.children() {
            self.push_node(child.clone(), Connection::DescendantOf(node.get_opaque()))?;
            self.write_all_children(child)?;
        }
        Ok(())
    }
}

pub trait GraphStyle: GetOpaqueNode {
    fn graph_style(&self) -> GraphDisplayStyle;
}

pub fn serialize_dot_graph<N: TreeLinked + GraphStyle, W: Write>(node: N, output: W) -> Result<()> {
    let mut serializer = DotGraphSerializer::new(output);
    serializer.write_dot(node)
}