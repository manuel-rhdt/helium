use super::tree::TreeLinked;

pub trait Traversal<NodeRef> {
    // called on top-down traversal
    fn preorder_traverse(&mut self, node: NodeRef);

    // called on bottom-up traversal
    fn postorder_traverse(&mut self, node: NodeRef);
}

pub trait Traverse<T> {
    fn traverse(&self, traversal: &mut T);
}

impl<T, Tree> Traverse<T> for Tree
where
    T: Traversal<Tree>,
    Tree: TreeLinked,
{
    fn traverse(&self, traversal: &mut T) {
        traversal.preorder_traverse(self.clone());
        self.children().for_each(|child| child.traverse(traversal));
        traversal.postorder_traverse(self.clone());
    }
}
