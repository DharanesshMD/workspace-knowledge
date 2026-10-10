export type Body = {schema_version:number; idempotency_key:string; policy_revision:number; title:string; intent:string; rationale:string; decisions:string[]; constraints:string[]; open_questions:string[]; service_ids:string[]; evidence:{kind:string; reference:string}[]; lifecycle:string; capture_completeness:string; base_revision:number|null};
export type Card = {id:string; body:Body; revision:number; published:boolean; publication_mode?:string; review_reason?:string; evidence_freshness?:string};
export type Status = {publication_mode?:string; inference_destination?:string; paused?:boolean; connectors?:{name:string; status:string; coverage?:string}[]; services?:string[]; policy_revision?:number};
export type Profile = {id:string; name:string; service_ids:string[]; access:string; variables:{name:string; required:boolean; description?:string}[]};
export type Edge = {source:string; target:string; relation:string};

export type PersonalSummary={id:string;body:Body;revision:number;provenance:'manual'|'captured';created_at_ms:number;expires_at_ms:number;submitted_card:string|null};

export type TeamEntry={card:Card;author:string;created_at_ms:number};
export type SharedActivity={developer:string;intent:string;files:string[];services:string[];reported_at_ms:number;expires_at_ms:number};
export type DeviceSettings={paused:boolean;owner_sync:boolean;activity:boolean};
export type Device={id:string;product:string;services:string[];settings:DeviceSettings;revision:number;revoked:boolean;created_at_ms:number;reported_at_ms:number|null;report:null|{collector_version:string;config_revision:number;config_status:'applied'|'rejected';local_pause:boolean;local_owner_sync:boolean;local_activity:boolean;queue:'unknown'|'empty'|'small'|'large';reason:string;capabilities:{client:string;surface:string;version:string|null;status:string}[]}};
