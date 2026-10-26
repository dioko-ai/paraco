import type {Dataset} from './paraco';
export function datasetClient(call: (request: object) => Promise<unknown>, dataset?: string): Dataset;
