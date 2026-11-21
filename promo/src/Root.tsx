import React from 'react';
import {Composition} from 'remotion';
import {Promo} from './Promo';
import type {PromoProps} from './plan';

// One composition; scripts/render.mjs passes the planned props (see plan.ts) as inputProps.
export const RemotionRoot: React.FC = () => (
  <Composition
    id="Promo"
    component={Promo as unknown as React.FC<Record<string, unknown>>}
    width={1080}
    height={1920}
    fps={30}
    durationInFrames={30}
    defaultProps={{} as Record<string, unknown>}
    calculateMetadata={({props}) => {
      const p = props as unknown as PromoProps;
      return {durationInFrames: Math.max(1, p.frames || 1), fps: p.fps || 30, width: p.width || 1080, height: p.height || 1920};
    }}
  />
);
