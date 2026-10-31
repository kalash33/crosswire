// Inline SVG icon set (U1 spec D2). Drawn on a 16x16 grid as 1.5 px strokes
// (`fill="none" stroke="currentColor"`), so an icon takes the colour of its
// text. Path data is static and trusted: it is part of the bundle, never
// derived from captured bytes. No icon package is used (no new dependency).

export type IconName =
  | 'history'
  | 'repeater'
  | 'intercept'
  | 'scope'
  | 'decoder'
  | 'settings'
  | 'send'
  | 'copy'
  | 'tag'
  | 'plus'
  | 'close'
  | 'search'
  | 'filter'
  | 'chevron-down'
  | 'chevron-right'
  | 'play'
  | 'pause'
  | 'forward'
  | 'drop'
  | 'hex'
  | 'raw'
  | 'headers'
  | 'sun'
  | 'moon'
  | 'keyboard'
  | 'external'
  | 'check'
  | 'warning';

const circle = (cx: number, cy: number, r: number) =>
  `M${cx - r} ${cy}a${r} ${r} 0 1 0 ${2 * r} 0a${r} ${r} 0 1 0 ${-2 * r} 0z`;

export const ICONS: Readonly<Record<IconName, readonly string[]>> = {
  history: [circle(8, 8, 5.75), 'M8 4.75V8l2.25 1.5'],
  repeater: ['M2.5 6h9l-2.25-2.25', 'M13.5 10h-9l2.25 2.25'],
  intercept: ['M1.75 8h7.5', 'M6.75 5.25 9.5 8l-2.75 2.75', 'M12.75 2.75v10.5'],
  scope: [circle(8, 8, 5), circle(8, 8, 1.75), 'M8 1v2.25M8 12.75V15M1 8h2.25M12.75 8H15'],
  decoder: ['M3.5 2.75v4.5', 'M8.75 2.75h3.5v4.5h-3.5z', 'M3.75 8.75h3.5v4.5h-3.5z', 'M12.5 8.75v4.5'],
  settings: ['M2 4.5h3M8 4.5h6', 'M2 11.5h7M12 11.5h2', circle(6.5, 4.5, 1.5), circle(10.5, 11.5, 1.5)],
  send: ['M14.25 1.75 7 9', 'M14.25 1.75 9.75 14.25 7 9 1.75 6.25z'],
  copy: ['M5.75 5.75h7.5v7.5h-7.5z', 'M10.25 5.75v-3h-7.5v7.5h3'],
  tag: ['M2.25 2.25h5.5l6 6-5.5 5.5-6-6z', 'M5.25 5.25h.01'],
  plus: ['M8 3v10M3 8h10'],
  close: ['M4 4l8 8M12 4l-8 8'],
  search: [circle(7, 7, 4.5), 'M10.25 10.25 14 14'],
  filter: ['M2 3h12L9.5 8.5v4.75l-3-1.5V8.5z'],
  'chevron-down': ['M4 6l4 4 4-4'],
  'chevron-right': ['M6 4l4 4-4 4'],
  play: ['M5 3l8 5-8 5z'],
  pause: ['M5.5 3.5v9M10.5 3.5v9'],
  forward: ['M2.5 8h9.5', 'M8.5 4.5 12 8l-3.5 3.5', 'M14 3.5v9'],
  drop: ['M2.75 4.5h10.5', 'M6.25 4.5V2.75h3.5V4.5', 'M4.25 4.5l.6 8.75h6.3l.6-8.75', 'M6.75 7v4M9.25 7v4'],
  hex: ['M8 1.5l5.5 3.25v6.5L8 14.5l-5.5-3.25v-6.5z'],
  raw: ['M3 4h10M3 8h10M3 12h6'],
  headers: ['M2.5 4h3M7.5 4h6M2.5 8h3M7.5 8h6M2.5 12h3M7.5 12h4'],
  sun: [
    circle(8, 8, 2.75),
    'M8 1.25v1.5M8 13.25v1.5M1.25 8h1.5M13.25 8h1.5',
    'M3.25 3.25l1.05 1.05M11.7 11.7l1.05 1.05M3.25 12.75l1.05-1.05M11.7 4.3l1.05-1.05',
  ],
  moon: ['M13.5 9.75A5.75 5.75 0 1 1 6.25 2.5a4.6 4.6 0 0 0 7.25 7.25z'],
  keyboard: ['M1.75 4.25h12.5v7.5H1.75z', 'M4.25 6.75h.01M6.75 6.75h.01M9.25 6.75h.01M11.75 6.75h.01', 'M5 9.25h6'],
  external: ['M9.5 2.5h4v4', 'M13.5 2.5 7.5 8.5', 'M11.75 9.5v4h-9.25V4.25h4'],
  check: ['M3 8.5l3 3 7-7'],
  warning: ['M8 2 14.5 13.5h-13z', 'M8 6.5v3', 'M8 11.5h.01'],
};
