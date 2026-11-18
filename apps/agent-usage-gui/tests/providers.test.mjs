import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";
import { JSDOM } from "jsdom";

const bundle = await build({
  entryPoints: [fileURLToPath(new URL("../src/main.ts", import.meta.url))],
  bundle: true, write: false, format: "iife", plugins: [{
    name: "tauri-test-host",
    setup(builder) {
      builder.onResolve({filter: /^@tauri-apps\/api\//}, args => ({path: args.path, namespace:"mock"}));
      builder.onLoad({filter: /.*/, namespace:"mock"}, () => ({contents: `
        export const invoke=(command,args)=>window.host.invoke(command,args);
        export const listen=async(event,callback)=>{window.host.listener=callback;return ()=>{};};
        export const getCurrentWindow=()=>({setSize:async()=>{},minimize:async()=>{},close:async()=>{}});
        export class LogicalSize { constructor(width,height){this.width=width;this.height=height;} }
      `}));
    },
  }],
});
const html = await readFile(new URL("../index.html", import.meta.url), "utf8");
const settle = () => new Promise(resolve => setImmediate(resolve));
async function setup(cli = {supported:true,available:true,enabled:false,managed:false,conflicts:[]}) {
  const dom = new JSDOM(html, {runScripts:"outside-only",url:"http://localhost"});
  const win = dom.window;
  const calls=[];
  let settings={providers:[{id:"entry-1",name:null,config:{kind:"codex_inference",options:{model:"test-model",effort:"low",auth_file:null}},generation:0,shown:true,auto_refresh:true,interval_seconds:300}],thresholds:{critical:90},warn_fast_polling:true,pinned:true};
  win.ResizeObserver=class {observe(){}};
  win.host={listener:null,invoke:async(command,args)=>{
    calls.push({command,args});
    if(command==="get_meta")return {providers:[{id:"codex",name:"Codex"},{id:"claude",name:"Claude"}],codex_efforts:["low","high"]};
    if(command==="get_cli_status") return structuredClone(cli);
    if(command==="set_cli_enabled"){ if(cli.conflicts.length && args.enabled) throw new Error("Independent CLI found"); cli.enabled=args.enabled; cli.managed=args.enabled; return structuredClone(cli); }
    if(command==="get_settings")return structuredClone(settings);
    if(command==="save_entry"){
      const draft=args.entry;
      const existing=settings.providers.find(p=>p.id===draft.id);
      if(existing){if(JSON.stringify(existing.config)!==JSON.stringify(draft.config))existing.generation++;existing.config=draft.config;existing.name=draft.name;}
      else settings.providers.push({...draft,id:"entry-2",generation:0,shown:true,auto_refresh:true,interval_seconds:300});
      return structuredClone(settings);
    }
    if(command==="remove_entry"){settings.providers=settings.providers.filter(p=>p.id!==args.id);return structuredClone(settings);}
    if(command==="save_preferences"){Object.assign(settings,args.preferences);return structuredClone(settings);}
  }};
  win.eval(bundle.outputFiles[0].text);
  await settle();
  const doc=win.document;
  const button=name=>[...doc.querySelectorAll("button")].find(b=>b.getAttribute("aria-label")===name || b.textContent===name);
  const input=label=>[...doc.querySelectorAll("label")].find(l=>l.firstElementChild?.textContent===label)?.querySelector("input,select");
  const change=(field,value)=>{field.value=value;field.dispatchEvent(new win.Event("change",{bubbles:true}));};
  const submit=async()=>{doc.querySelector("form").dispatchEvent(new win.Event("submit",{bubbles:true,cancelable:true}));await settle();};
  return {dom,win,doc,calls,button,input,change,submit};
}

test("provider editor shows only supported types and strategy-specific settings",async()=>{
  const t=await setup();try{
    t.button("Settings").click();t.button("Add provider").click();
    assert.equal(t.input("Name (optional)").value,"");
    assert.deepEqual([...t.input("Provider").options].map(o=>o.value),["codex","claude"]);
    assert.ok(t.input("Model"));assert.equal(t.input("Codex executable"),undefined);
    t.change(t.input("Strategy"),"account");
    assert.ok(t.input("Codex executable"));assert.equal(t.input("Model"),undefined);
    assert.equal(t.doc.querySelector(".interval"),null);
    t.input("Name (optional)").value="My account";
    await t.submit();
    const saved=t.calls.find(c=>c.command==="save_entry").args.entry;
    assert.equal(saved.config.kind,"codex_account");assert.equal(saved.name,"My account");
    assert.equal(saved.config.options.model,undefined);
    assert.ok(t.doc.querySelector(".provider-entry").textContent.includes("Codex"));
  }finally{t.dom.window.close();}
});

test("cancel does not save; renaming does not refresh; remove clears the entry",async()=>{
  const t=await setup();try{
    t.button("Settings").click();t.doc.querySelector(".provider-entry").click();
    t.input("Name (optional)").value="Discard";t.button("Cancel").click();
    assert.equal(t.calls.filter(c=>c.command==="save_entry").length,0);
    t.doc.querySelector(".provider-entry").click();t.input("Name (optional)").value="Work";await t.submit();
    assert.equal(t.calls.filter(c=>c.command==="refresh").length,0);
    t.doc.querySelector(".provider-entry").click();t.button("Remove").click();await settle();
    assert.equal(t.doc.querySelectorAll(".provider-entry").length,0);
  }finally{t.dom.window.close();}
});

test("obsolete reports are ignored and elapsed reset times retain measured usage",async()=>{
  const t=await setup();try{
    const report={provider:"codex",plan:null,metrics:[{kind:"window",label:"Test",used_percent:42,remaining_percent:58,duration_minutes:300,resets_at:1}],limit_reached:false,details:{},fetched_at:0,error:null};
    t.win.host.listener({payload:{entry_id:"entry-1",generation:99,report,source_report:null}});
    assert.ok(!t.doc.querySelector(".card-body").textContent.includes("42%"));
    t.win.host.listener({payload:{entry_id:"entry-1",generation:0,report,source_report:null}});
    assert.ok(t.doc.querySelector(".card-body").textContent.includes("42%"));
    assert.ok(t.doc.querySelector(".card-body").textContent.includes("awaiting refresh"));
    t.win.host.listener({payload:{entry_id:"entry-1",generation:0,report:{...report,error:"Offline"},source_report:null}});
    assert.ok(t.doc.querySelector(".card").classList.contains("stale"));
    assert.ok(t.doc.querySelector(".card-body").textContent.includes("42%"));
  }finally{t.dom.window.close();}
});

test("CLI conflict disables enabling and Check again rechecks without changing installations", async()=>{
  const cli={supported:true,available:true,enabled:false,managed:false,conflicts:["C:\\Tools\\agent-usage-cli.exe"]};
  const t=await setup(cli);try{
    t.button("Settings").click();await settle();
    const toggle=t.doc.querySelector('[aria-label="Enable command line interface"]');
    assert.equal(toggle.disabled,true);
    assert.ok(t.doc.querySelector(".cli-conflicts").textContent.includes(cli.conflicts[0]));
    assert.equal(t.calls.filter(c=>c.command==="set_cli_enabled").length,0);
    cli.conflicts=[];t.button("Check again").click();await settle();
    assert.equal(toggle.disabled,false);
    toggle.click();await settle();
    assert.equal(toggle.checked,true);
    assert.deepEqual(t.calls.filter(c=>c.command==="set_cli_enabled").map(c=>c.args.enabled),[true]);
    cli.conflicts=["C:\\Other\\agent-usage-cli.exe"];
    t.button("Check again").click();await settle();
    assert.equal(toggle.disabled,false); // still allow removing our own registration
    toggle.click();await settle();
    assert.equal(toggle.checked,false);assert.equal(toggle.disabled,true);
  }finally{t.dom.window.close();}
});

test("CLI enable failure refreshes the conflict warning and restores the checkbox", async()=>{
  const cli={supported:true,available:true,enabled:false,managed:false,conflicts:[]};
  const t=await setup(cli);try{
    t.button("Settings").click();await settle();
    const toggle=t.doc.querySelector('[aria-label="Enable command line interface"]');
    cli.conflicts=["C:\\Tools\\agent-usage-cli.exe"];
    toggle.click();await settle();
    assert.equal(toggle.checked,false);assert.equal(toggle.disabled,true);
    assert.match(t.doc.querySelector(".cli-settings .form-error").textContent,/Independent CLI/);
  }finally{t.dom.window.close();}
});
