const nn=process.env.CODEMODE_TRIAL_NUDGE_DB,rr=process.env.CODEMODE_TRIAL_RESEARCHER_DB;
if (typeof nn !== 'string' || typeof rr !== 'string' || !/^codemode_spike_/.test(nn) || !/^codemode_spike_/.test(rr)) throw new Error('disposable database names required');
const n=db.getSiblingDB(nn),r=db.getSiblingDB(rr);

n.teams.insertOne({id:'trial-team',workspace_id:'workspace_default',key:'CM',name:'Codemode',estimates_enabled:false,estimate_scale:'fibonacci',estimate_extended:false,estimate_allow_zero:false,estimate_unestimated_as_one:false,estimate_policy_revision:Long(1),created_at_ms:Long(Date.now())});
n.projects.insertOne({id:'trial-project',workspace_id:'workspace_default',key:'gateway',name:'Gateway',description:'Disposable trial project',color:'#0a6c7e',status:'started',created_at_ms:Long(Date.now()),updated_at_ms:Long(Date.now())});
n.project_teams.insertOne({id:'trial-project-team',project_id:'trial-project',team_id:'trial-team',created_at_ms:Long(Date.now())});
n.issue_statuses.insertOne({id:'trial-open',team_id:'trial-team',name:'Open',category:'unstarted',color:'#888888',position:Long(0),created_at_ms:Long(Date.now())});
for(let i=1;i<=3;i++)n.issues.insertOne({id:'trial-issue-'+i,identifier:'CM-'+i,team_id:'trial-team',project_id:'trial-project',status_id:'trial-open',title:['Worker isolation','Snapshot restore','OAuth linking'][i-1],description:'Disposable Codemode trial fixture',search_text:'gateway codemode worker snapshot oauth',priority:Long(i),labels:[],position:Long(i),created_at_ms:Long(Date.now()),updated_at_ms:Long(Date.now())});
r.venues.insertOne({_id:'trial-conf',key:'trial-conf',name:'Codemode Test Conference',kind:'talk',tier:3,scholarly_grade:'N/A',applying:true,url:'https://example.invalid/trial',revision:1});
r.tracks.insertOne({_id:'trial-conf/talk',key:'talk',venue_key:'trial-conf',name:'Talk',lane:'talk',kind:'talk'});
r.inventory.insertOne({_id:'gateway-demo',kind:'talk',title:'Gateway sandbox demo',summary:'Disposable trial talk',updated_at:new Date(),updated_by:'fixture'});
print('Seeded only disposable trial content');
