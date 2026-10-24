//! Graph document rules and persistence shared by desktop hosts.
use std::{fs,path::{Path,PathBuf},sync::Mutex};
use chrono::{Utc,SecondsFormat};
use super::types::{Automation,SCHEMA_VERSION,is_allowed_node_type};
static WRITES:Mutex<()>=Mutex::new(());
pub fn validate_id(id:&str)->Result<(),String>{
    if id.is_empty()||id.contains('/')||id.contains('\\')||id.contains("..") {Err("invalid automation id".into())}else{Ok(())}
}
pub fn path(root:&Path,id:&str)->Result<PathBuf,String>{validate_id(id)?;Ok(root.join(format!("{id}.json")))}
pub fn get(root:&Path,id:&str)->Result<Automation,String>{super::catalog::read_one(&path(root,id)?)}
pub fn validate_graph(automation:&Automation)->Result<(),String>{
    for node in &automation.nodes{validate_id(&node.id)?;if !is_allowed_node_type(&node.node_type){return Err(format!("unknown node type: {}",node.node_type));}}
    for edge in &automation.edges{validate_id(&edge.id)?;validate_id(&edge.source)?;validate_id(&edge.target)?;}
    if automation.enabled{super::validate::ensure_runnable(automation)?;}Ok(())
}
fn merge(saved:&mut serde_json::Value,next:&serde_json::Value){
    if let (Some(old),Some(new))=(saved.as_object_mut(),next.as_object()){
        for(k,v)in new{if let Some(existing)=old.get_mut(k){merge(existing,v)}else{old.insert(k.clone(),v.clone());}}
    }else if let (Some(old),Some(new))=(saved.as_array(),next.as_array()){
        let mut items=vec![];for v in new{let mut item=v.clone();if let Some(id)=v.get("id"){if let Some(previous)=old.iter().find(|o|o.get("id")==Some(id)){item=previous.clone();merge(&mut item,v);}}items.push(item);}*saved=serde_json::Value::Array(items);
    }else{*saved=next.clone();}
}
pub fn save(root:&Path,mut automation:Automation)->Result<Automation,String>{
    let _guard=WRITES.lock().unwrap_or_else(|e|e.into_inner());
    if automation.id.trim().is_empty(){automation.id=uuid::Uuid::new_v4().to_string();}
    validate_id(&automation.id)?;automation.schema_version=SCHEMA_VERSION;
    if automation.created_at.trim().is_empty(){automation.created_at=Utc::now().to_rfc3339_opts(SecondsFormat::Secs,true);}
    automation.updated_at=Utc::now().to_rfc3339_opts(SecondsFormat::Secs,true);
    validate_graph(&automation)?;
    let path=path(root,&automation.id)?;
    let mut saved=match fs::read(&path){Ok(bytes)=>serde_json::from_slice(&bytes).map_err(|e|e.to_string())?,Err(e)if e.kind()==std::io::ErrorKind::NotFound=>serde_json::json!({}),Err(e)=>return Err(e.to_string())};
    let next=serde_json::to_value(&automation).map_err(|e|e.to_string())?;merge(&mut saved,&next);
    crate::chat::storage::atomic_write(&path,&serde_json::to_string_pretty(&saved).map_err(|e|e.to_string())?,"automation")?;Ok(automation)
}
pub fn delete(root:&Path,id:&str)->Result<(),String>{
    let _guard=WRITES.lock().unwrap_or_else(|e|e.into_inner());
    match fs::remove_file(path(root,id)?){Ok(())=>Ok(()),Err(e)if e.kind()==std::io::ErrorKind::NotFound=>Ok(()),Err(e)=>Err(e.to_string())}
}
pub fn import(root:&Path,source:&Path)->Result<Automation,String>{
    let mut graph=super::catalog::read_one(source)?;graph.id=uuid::Uuid::new_v4().to_string();graph.enabled=false;graph.created_at=String::new();save(root,graph)
}
pub fn export(root:&Path,id:&str,target:&Path)->Result<(),String>{
    get(root,id)?;let source=path(root,id)?;
    if source.canonicalize().ok()==target.canonicalize().ok(){return Ok(());}
    let bytes=fs::read(source).map_err(|e|e.to_string())?;
    crate::chat::storage::atomic_write(target,&String::from_utf8(bytes).map_err(|e|e.to_string())?,"automation export")
}
