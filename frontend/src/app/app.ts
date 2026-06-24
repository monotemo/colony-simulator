import { Component, ChangeDetectionStrategy } from '@angular/core';
import { Controls } from './controls';
import { WorldCanvas } from './world-canvas';

@Component({
  selector: 'app-root',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Controls, WorldCanvas],
  templateUrl: './app.html',
  styleUrl: './app.scss',
})
export class App {}
