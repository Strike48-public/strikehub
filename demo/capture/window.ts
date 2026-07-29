export type Win = {
  address: string;
  monitor: number;
  x: number;
  y: number;
  w: number;
  h: number;
};

export function findStrikehub(clientsJson: string): Win | null {
  const clients = JSON.parse(clientsJson) as any[];
  const c = clients.find((cl) => cl.class === "Strikehub");
  if (!c) return null;
  return {
    address: c.address,
    monitor: c.monitor,
    x: c.at[0],
    y: c.at[1],
    w: c.size[0],
    h: c.size[1],
  };
}

export function placeCmds(address: string, headless: string): string[] {
  return [
    `hyprctl --batch "dispatch focuswindow address:${address} ; dispatch movewindow mon:${headless}"`,
    "hyprctl dispatch fullscreen 1",
  ];
}
