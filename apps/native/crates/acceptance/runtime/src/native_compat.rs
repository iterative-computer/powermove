//! Real compat V8 → native kernel/recorder path. No synthetic host implementation.
use pm_compat_runner::{CompatRunner, initial};
use pm_runtime_v8::{Error, Result};
use serde_json::json;
pub const IDS: &[&str] = &[
    "A690",
    "W040.compat-kernel-history",
    "W040.compat-kernel-edits",
    "W040.compat-native-pixels",
];
pub fn check(id: &str) -> Result<()> {
    let runner = CompatRunner::new(&initial())?;
    match id {
        "A690" => {
            for version in 1..=3 {
                let manifest = json!({"id":"synthetic","name":"Synthetic","version":"1.0.0","apiVersion":version});
                let source = r#"export default function(api){
                    globalThis.api=api;
                    const round = (api.storage.get('round')||0)+1;
                    api.storage.set('round',round);
                    api.storage.set('authored',{text:'λ',nested:[null,true,1.23456789012345]});
                }"#;
                assert_eq!(runner.activate(&manifest, source)?["ok"], true);
                assert_eq!(
                    runner.json("[api.storage.get('round'),api.storage.get('authored')]")?,
                    json!([version,{"text":"λ","nested":[null,true,1.23456789012345]}])
                );
                assert_eq!(runner.json("(()=>{const other=__compat.realm.extension('isolated');other.api.storage.set('authored','other owner');const isolated=other.api.storage.get('authored');api.storage.delete('authored');api.storage.delete('missing');const absent=api.storage.get('authored')===undefined;other.dispose();return [isolated,absent,storage['ext.isolated'].authored]})()")?,json!(["other owner",true,"other owner"]));
            }
        }
        "W040.compat-kernel-history" => {
            assert_eq!(runner.json("(()=>{const before=PM.proj.name;const value=__compat.realm.history.do('Synthetic',()=>{PM.proj.name='Edited';return {exact:1.23456789012345}});const edited=PM.proj.name;const undo=PM.hist.undo();const restored=PM.proj.name===before;const redo=PM.hist.redo();return [value,edited,undo,restored,redo,PM.proj.name]})()")?,json!([{"exact":1.23456789012345},"Edited",true,true,true,"Edited"]));
            assert_eq!(runner.json("(()=>{let count=0;__compat.realm.history.external('Owned closure',()=>count--,()=>count++);return [PM.hist.undo(),count,PM.hist.redo(),count]})()")?,json!([true,-1,true,0]));
            assert_eq!(runner.json("(()=>{const e={exact:'throw'};let same=false;try{__compat.realm.history.do('Refused',()=>{PM.proj.name='Must rollback';throw e})}catch(x){same=x===e}return [same,PM.proj.name]})()")?,json!([true,"Edited"]));
            assert_eq!(runner.json("(()=>{const h=__compat.realm.history;h.begin('Kept writes');PM.proj.name='Kept after cancel';__compat.realm.flush();h.cancel();return [PM.proj.name,PM.hist.busy()]})()")?,json!(["Kept after cancel",false]));
        }
        "W040.compat-kernel-edits" => {
            assert_eq!(runner.json("(()=>{const edit=__compat.realm.edit;const added=edit.apply({type:'add_layer',id:'native-layer',layerType:'solid'});edit.begin('Live');const dispatched=edit.dispatch({type:'set_layer',target:'native-layer',patch:{name:'During drag'}});const live=PM.proj.layers[0].name;const cancelled=edit.cancel();const restored=PM.proj.layers[0].name!=='During drag';edit.begin('Committed');edit.dispatch({type:'set_layer',target:'native-layer',patch:{name:'Committed'}});const committed=edit.commit();const undo=PM.hist.undo();const undone=PM.proj.layers[0].name!=='Committed';const redo=PM.hist.redo();return [added.ok,dispatched.ok,live,cancelled,restored,committed.ok,undo,undone,redo,PM.proj.layers[0].name]})()")?,json!([true,true,"During drag",true,true,true,true,true,true,"Committed"]));
            assert_eq!(runner.json("(()=>{const before=JSON.stringify(PM.proj);const result=__compat.realm.edit.apply([{type:'set_layer',target:'native-layer',patch:{name:'Must not apply'}},{type:'set_layer',target:'missing',patch:{name:'Invalid'}}]);return [result.ok,JSON.stringify(PM.proj)===before]})()")?,json!([false,true]));
        }
        "W040.compat-native-pixels" => {
            assert_eq!(runner.json("(()=>{const a=__compat.realm.createCanvas(2,2),b=__compat.realm.createCanvas(2,2);const c=a.getContext('2d');c.fillStyle='#ff0000';c.fillRect(0,0,2,2);const target=b.getContext('2d');target.drawImage(a,0,0);a.width=4;a.dispose();const pixels=target.getImageData(0,0,2,2).data;b.dispose();return pixels})()")?,json!([255,0,0,255,255,0,0,255,255,0,0,255,255,0,0,255]));
        }
        _ => return Err(Error::Invalid("unknown connected compat case")),
    }
    let missing = runner.diagnostics()["missing"].clone();
    assert_eq!(
        missing,
        json!([]),
        "a connected case requested an unattached backing"
    );
    Ok(())
}
