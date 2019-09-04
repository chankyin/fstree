#![feature(async_closure)]

use std::cell::RefCell;
use std::fs::Metadata;
use std::future::Future;
use std::io::Result;
use std::path::PathBuf;
use std::pin::Pin;

use tokio::fs;
use tokio_executor::blocking;

#[tokio::main]
async fn main() -> Result<()> {
    let mut root = std::env::current_dir()?;
    while let Some(parent) = root.parent() {
        root = parent.clone().to_path_buf();
    }
    dbg!(&root);

    let _tree = Tree::new(root).await;

    Ok(())
}

async fn real_size(path: PathBuf, metadata: Metadata) -> Result<u64> {
    blocking::run(move || filesize::file_real_size_fast(&path, &metadata)).await
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Tree {
    files: u64,
    dirs: u64,
    links: u64,
    others: u64,
    errors: u64,
    size: u64,
    local_error_log: Vec<String>,
    children: Vec<Tree>,
}

impl Tree {
    pub fn new(path: PathBuf) -> Pin<Box<dyn Future<Output = Tree>>> {
        Box::pin(async move {
            // do not follow symlinks
            match fs::symlink_metadata(path.clone()).await {
                Ok(metadata) => {
                    let ty = metadata.file_type();

                    let mut tree = Tree::default();
                    tree.size = match real_size(path.clone(), metadata.clone()).await {
                        Ok(size) => size,
                        Err(err) => {
                            tree.errors += 1;
                            tree.local_error_log.push(err.to_string());
                            return tree;
                        }
                    };
                    if ty.is_file() {
                        tree.files = 1;
                    } else if ty.is_symlink() {
                        tree.links = 1;
                    } else if ty.is_dir() {
                        use futures_util::stream::StreamExt;

                        let dir_list = fs::read_dir(path).await;
                        match dir_list {
                            Ok(dir_list) => {
                                let dir_list = dir_list.collect::<Vec<Result<_>>>().await;
                                let mut futures = vec![];
                                let cell = RefCell::new(tree);
                                for entry in dir_list {
                                    let mut tree = cell.borrow_mut();
                                    let future = async {
                                        match entry {
                                            Ok(entry) => {
                                                tree.dirs += 1;
                                                let child = Tree::new(entry.path()).await;
                                                *tree += &child;
                                                tree.children.push(child);
                                            }
                                            Err(err) => {
                                                tree.errors += 1;
                                                tree.local_error_log.push(err.to_string());
                                            }
                                        }
                                    };
                                    futures.push(future);
                                }
                                futures_util::future::join_all(futures).await;
                                tree = cell.into_inner();
                            }
                            Err(err) => {
                                tree.errors += 1;
                                tree.local_error_log.push(err.to_string());
                            }
                        }
                    } else {
                        tree.others = 1;
                    }

                    tree
                }
                Err(err) => Tree {
                    errors: 1,
                    local_error_log: vec![err.to_string()],
                    ..Default::default()
                },
            }
        })
    }
}

impl<'a> std::ops::AddAssign<&'a Tree> for Tree {
    fn add_assign(&mut self, rhs: &'a Tree) {
        self.files += rhs.files;
        self.dirs += rhs.dirs;
        self.links += rhs.links;
        self.others += rhs.others;
        self.errors += rhs.errors;
        self.size += rhs.size;
    }
}
