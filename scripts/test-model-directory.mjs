import assert from 'node:assert/strict';
import { configuredModels, modelDirectoryHtml, configuredInterfaces, interfaceDirectoryHtml } from '../app/ui/js/models-page.js';

const primary = { id: 'one', name: 'Primary', model: 'Model', provider_id: 'Provider', base_url: 'https://example.invalid/v1', wire_api: 'responses' };
const profiles = [primary, { ...primary, id: 'two', name: 'Alternate' }, { ...primary, id: 'three', base_url: 'https://other.invalid/v1' }];
const status = { configured_model: 'Model', configured_provider: 'Provider', current_draft: primary };
const rows = configuredModels(status, profiles);
assert.equal(rows.length, 2, 'same model on different endpoints must remain distinct');
assert.equal(rows[0].current, true);
assert.equal(rows[0].profiles.length, 2, 'profiles sharing a target are grouped');
assert.equal(rows[1].current, false);
assert.equal(configuredModels(status, [...profiles, { ...primary, wire_api: 'chat' }]).length, 3, 'protocols remain distinct');
assert.equal(configuredModels({ ...status, config_error: 'invalid' }, []).length, 0, 'failed config cannot supply a current target');
assert.equal(configuredModels({ ...status, config_error: 'invalid' }, profiles).some(row => row.current), false);
assert.match(modelDirectoryHtml(status, [], false), /目录可能不完整/);
assert.doesNotMatch(modelDirectoryHtml(status, []), /目录可能不完整/);
assert.match(modelDirectoryHtml({}, []), /尚未指定模型/);
const unsafeName = modelDirectoryHtml({}, [{ ...primary, model: '<script>alert(1)</script>' }]);
assert.match(unsafeName, /&lt;script&gt;/);
assert.doesNotMatch(unsafeName, /<script>/);


assert.equal(rows[0].choices.length, 2);
assert.equal(rows[0].choices[0].id, 'one');
assert.match(modelDirectoryHtml({ ...status, revision:'fixture' }, profiles), /data-model-profile="one"/);
assert.match(modelDirectoryHtml({ ...status, revision:'fixture', config_error:'invalid' }, profiles), /data-model-profile="one" disabled/);
assert.match(modelDirectoryHtml({ ...status, revision:'fixture' }, profiles, false), /data-model-profile="one" disabled/);
assert.match(modelDirectoryHtml({}, [{ ...primary, id:'<unsafe"', name:'<unsafe>' }]), /data-model-profile="&lt;unsafe&quot;" disabled/);
console.log('PASS: grouped targets, stable profile choices, read failure protection and escaped actions');

assert.match(modelDirectoryHtml({ revision: "fixture" }, [{ ...primary, id: "legacy", wire_api: "chat" }]), /data-model-profile="legacy" disabled/);
assert.match(modelDirectoryHtml({ revision: "fixture" }, [{ ...primary, id: "legacy", wire_api: "chat" }]), /旧档位，需编辑协议/);

const identityProfiles=[{...primary,env_key:'FIXTURE_AUTH_A'},{...primary,id:'auth-b',env_key:'FIXTURE_AUTH_B'},{...primary,id:'other-model',model:'Second',env_key:'FIXTURE_AUTH_A'},{...primary,id:'old-chat',wire_api:'chat',env_key:'FIXTURE_AUTH_A'}];
const identityStatus={...status,current_draft:identityProfiles[0],revision:'fixture'};
assert.equal(configuredModels(identityStatus,identityProfiles).length,4,'authentication references are part of model identity');
const interfaces=configuredInterfaces(identityStatus,identityProfiles);
assert.equal(interfaces.length,3,'different protocol or authentication references never merge');
assert.deepEqual(interfaces[0].models,['Model','Second']);
assert.equal(interfaces[0].current,true);
assert.equal(interfaces[0].choices.length,2);
assert.equal(configuredInterfaces({...identityStatus,config_error:'invalid'},identityProfiles).some(i=>i.current),false);
const escaped=interfaceDirectoryHtml(identityStatus,[{...primary,name:'<unsafe>',env_key:'<ref>',provider_id:'<provider>',base_url:'https://example.invalid/"'}]);
assert.doesNotMatch(escaped,/<unsafe>|<ref>|<provider>/);
assert.match(escaped,/&lt;ref&gt;/);
assert.match(interfaceDirectoryHtml(identityStatus,[identityProfiles[3]]),/data-model-profile="old-chat" disabled/);
assert.match(modelDirectoryHtml(identityStatus,identityProfiles,true,'interfaces','"fixture'),/data-catalog-kind="interfaces"/);
assert.match(modelDirectoryHtml(identityStatus,identityProfiles,true,'interfaces','"fixture'),/value="&quot;fixture"/);
assert.match(modelDirectoryHtml(identityStatus,identityProfiles),/data-catalog-search/);
console.log('PASS: interface identity includes protocol and auth reference, preserves model choices, rejects false current targets, escapes search and actions');
