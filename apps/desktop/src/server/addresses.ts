import { networkInterfaces } from 'node:os';

/* The URLs `powermove serve` prints: loopback first, then Tailscale, then LAN. */

export function isTailnetAddress(address: string): boolean {
  const [a, b] = address.split('.').map(Number);
  return a === 100 && b !== undefined && b >= 64 && b <= 127;
}

export function reachableAddresses(host: string, port: number, scheme: 'http' | 'https' = 'http'): string[] {
  const urls: string[] = [];
  const add = (address: string) => { const url = `${scheme}://${address.includes(':') ? `[${address}]` : address}:${port}`; if (!urls.includes(url)) urls.push(url); };
  if (host !== '0.0.0.0' && host !== '::' && host !== '') { add(host); return urls; }
  add('localhost');
  const tailscale: string[] = [], lan: string[] = [];
  for (const entries of Object.values(networkInterfaces())) {
    for (const entry of entries ?? []) {
      if (entry.internal || entry.family !== 'IPv4') continue;
      // Tailscale hands out 100.64.0.0/10; surface those first.
      (isTailnetAddress(entry.address) ? tailscale : lan).push(entry.address);
    }
  }
  for (const address of [...tailscale, ...lan]) add(address);
  return urls;
}

