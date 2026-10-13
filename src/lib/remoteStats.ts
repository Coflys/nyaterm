import type { RemoteStats } from "@/types/global";
import { invoke } from "./invoke";

const inFlightRequests = new Map<string, Promise<RemoteStats>>();

/**
 * 获取远程资源信息，并合并同一会话尚未完成的请求。
 * 请求完成后不会缓存结果，下一次调用仍会获取最新数据。
 */
export function getRemoteStats(sessionId: string): Promise<RemoteStats> {
  const pending = inFlightRequests.get(sessionId);
  if (pending) return pending;

  const request = invoke<RemoteStats>("get_remote_stats", { sessionId });
  inFlightRequests.set(sessionId, request);

  const clearRequest = () => {
    if (inFlightRequests.get(sessionId) === request) {
      inFlightRequests.delete(sessionId);
    }
  };
  request.then(clearRequest, clearRequest);

  return request;
}
