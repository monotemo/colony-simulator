import {
  ApplicationConfig,
  provideBrowserGlobalErrorListeners,
  provideZoneChangeDetection,
  isDevMode,
  inject,
  Type,
} from '@angular/core';
import { provideServiceWorker } from '@angular/service-worker';
import { environment } from '../environments/environment';
import { SimulationService } from './simulation.service';
import { WebSocketSimulationService } from './websocket-simulation';
import { WasmSimulationService } from './wasm-simulation';

export const appConfig: ApplicationConfig = {
  providers: [
    provideBrowserGlobalErrorListeners(),
    provideZoneChangeDetection({ eventCoalescing: true }),
    provideServiceWorker('ngsw-worker.js', {
      enabled: !isDevMode(),
      registrationStrategy: 'registerWhenStable:30000',
    }),
    // Pick the simulation transport at build time: WASM in production
    // (GitHub Pages, no backend), WebSocket in development.
    {
      provide: SimulationService,
      useFactory: () => {
        const impl: Type<SimulationService> = environment.useWasm
          ? WasmSimulationService
          : WebSocketSimulationService;
        return inject(impl);
      },
    },
  ],
};
