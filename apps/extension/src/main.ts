import { bootstrapApplication } from '@angular/platform-browser';
import { appConfig } from './app/app.config';
import { App } from './app/app';
import { followTheme } from './shared/theme';

// Avant le premier rendu, pour ne pas montrer l'autre thème un instant.
followTheme();

bootstrapApplication(App, appConfig).catch((err) => console.error(err));
