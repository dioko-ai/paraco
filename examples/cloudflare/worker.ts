// Core logic is identical to `paraco run examples/hello`.
import app from '../hello/main';
import { cloudflare } from '../../runtime/cloudflare';
export default cloudflare(app);
