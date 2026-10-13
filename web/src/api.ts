export class ApiError extends Error { constructor(public status:number) { super(status === 401 ? 'Your session is not authorized. Reconnect with a valid token.' : status === 403 ? 'You do not have permission for this action or this console origin is not approved.' : status === 404 ? 'This record or capability is unavailable.' : status === 409 ? 'This record changed. Reload the latest revision before saving your change.' : status === 422 ? 'The server rejected these fields. Check the form and policy requirements.' : 'The service could not complete this request. Try again.'); } }
let sessionCsrf = '';
export function setSessionCsrf(value:string) { sessionCsrf=value; }
export async function request<T>(token:string,path:string,body?:unknown,method?:string,signal?:AbortSignal):Promise<T> {
  const response = await fetch(path, {method:method ?? (body === undefined ? 'GET':'POST'), headers:{...(token?{Authorization:`Bearer ${token}`}:{...(sessionCsrf?{'X-CSRF-Token':sessionCsrf}:{})}), ...(body === undefined ? {} : {'Content-Type':'application/json'})},body:body === undefined ? undefined : JSON.stringify(body), credentials:token?'omit':'same-origin', cache:'no-store', redirect:'error',signal});
  if (!response.ok) { if(response.status===401&&!token) window.dispatchEvent(new Event('wk-session-expired')); throw new ApiError(response.status); }
  return response.json() as Promise<T>;
}
export const productPath = (product:string) => `/v1/products/${encodeURIComponent(product)}`;
