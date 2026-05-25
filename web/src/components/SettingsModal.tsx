import {
  Alert,
  Button,
  Collapse,
  Form,
  Input,
  InputNumber,
  Modal,
  Select,
  Space,
  Tabs
} from '@arco-design/web-react';

const TabPane = Tabs.TabPane;

export type Settings = {
  lastFileRoot: string;
  aiEnabled: boolean;
  aiBaseUrl: string;
  aiApiKey: string;
  aiModel: string;
  ocrBaseUrl: string;
  ocrApiKey: string;
  requestTimeoutSeconds: number;
  imageMaxKb: number;
  themeMode: string;
  account: string;
  password: string;
};

const themeOptions = [
  { label: '白天', value: 'light' },
  { label: '跟随系统', value: 'system' },
  { label: '夜间', value: 'dark' }
];

type Props = {
  visible: boolean;
  draft: Settings;
  isSaving: boolean;
  onClose: () => void;
  onSave: () => void;
  onPatch: <K extends keyof Settings>(key: K, value: Settings[K]) => void;
  onChooseFolder: () => void;
};

export function SettingsModal({ visible, draft, isSaving, onClose, onSave, onPatch, onChooseFolder }: Props) {
  return (
    <Modal
      className="settings-modal"
      title="配置中心"
      visible={visible}
      maskClosable={false}
      style={{ width: 760 }}
      onCancel={onClose}
      footer={(
        <Space>
          <Button disabled={isSaving} onClick={onClose}>取消</Button>
          <Button type="primary" loading={isSaving} onClick={onSave}>保存设置</Button>
        </Space>
      )}
    >
      <Tabs type="capsule" size="small" destroyOnHide={false} lazyload={false} className="settings-tabs">
        <TabPane key="basic" title="基础设置">
          <div className="settings-pane">
            <Form layout="horizontal" labelAlign="left" labelCol={{ span: 5 }} wrapperCol={{ span: 19 }} colon={false}>
              <Form.Item label="工作目录">
                <Input
                  value={draft.lastFileRoot}
                  onChange={(value) => onPatch('lastFileRoot', value)}
                  addAfter={<Button size="small" type="primary" onClick={onChooseFolder}>选择</Button>}
                />
              </Form.Item>
              <Form.Item label="背景模式">
                <Select value={draft.themeMode} options={themeOptions} onChange={(value) => onPatch('themeMode', String(value))} />
              </Form.Item>
            </Form>
          </div>
        </TabPane>
        <TabPane key="hollysys" title="和利时账号">
          <div className="settings-pane">
            <Alert
              type="info"
              style={{ marginBottom: 12 }}
              content="保存后,Mac 会在内置登录页自动填入账号密码;Windows 会打开独立 Edge/Chrome 登录窗口,登录完成后回到应用刷新会话。账号密码会保存在本机 SQLite 配置中。"
            />
            <Form layout="horizontal" labelAlign="left" labelCol={{ span: 5 }} wrapperCol={{ span: 19 }} colon={false}>
              <Form.Item label="账号">
                <Input
                  value={draft.account}
                  placeholder="和利时统一身份认证用户名"
                  onChange={(value) => onPatch('account', value)}
                />
              </Form.Item>
              <Form.Item label="密码">
                <Input
                  type="password"
                  value={draft.password}
                  placeholder="留空则需要手动输入"
                  onChange={(value) => onPatch('password', value)}
                />
              </Form.Item>
            </Form>
          </div>
        </TabPane>
        <TabPane key="recognition" title="识别设置">
          <div className="settings-pane">
            <Alert
              type="info"
              style={{ marginBottom: 12 }}
              content="验收报告的手写签名、电话需要通过 AI 接口识别，请填写下方接口信息。"
            />
            <Form layout="horizontal" labelAlign="left" labelCol={{ span: 5 }} wrapperCol={{ span: 19 }} colon={false}>
              <Form.Item label="接口地址" extra="兼容 OpenAI 接口的服务地址">
                <Input
                  value={draft.aiBaseUrl}
                  placeholder="例如 https://api.openai.com/v1"
                  onChange={(value) => onPatch('aiBaseUrl', value)}
                />
              </Form.Item>
              <Form.Item label="接口密钥">
                <Input
                  type="password"
                  value={draft.aiApiKey}
                  placeholder="服务商提供的 API Key"
                  onChange={(value) => onPatch('aiApiKey', value)}
                />
              </Form.Item>
              <Form.Item label="模型名称" extra="需支持图片识别的多模态模型">
                <Input
                  value={draft.aiModel}
                  placeholder="例如 gpt-4o / qwen-vl-max"
                  onChange={(value) => onPatch('aiModel', value)}
                />
              </Form.Item>
              <Collapse bordered={false} style={{ background: 'transparent' }}>
                <Collapse.Item
                  name="advanced"
                  header="高级设置（一般无需修改）"
                >
                  <Form.Item label="独立 OCR 地址" extra="留空则使用上方接口地址">
                    <Input
                      value={draft.ocrBaseUrl}
                      placeholder="若 OCR 走另一家服务再填"
                      onChange={(value) => onPatch('ocrBaseUrl', value)}
                    />
                  </Form.Item>
                  <Form.Item label="独立 OCR 密钥">
                    <Input
                      type="password"
                      value={draft.ocrApiKey}
                      placeholder="留空则复用上方密钥"
                      onChange={(value) => onPatch('ocrApiKey', value)}
                    />
                  </Form.Item>
                  <Form.Item label="请求超时" extra="单位：秒，网络差时可调大">
                    <InputNumber
                      min={1}
                      max={300}
                      value={draft.requestTimeoutSeconds}
                      onChange={(value) => onPatch('requestTimeoutSeconds', Number(value || 30))}
                    />
                  </Form.Item>
                  <Form.Item label="图片压缩上限" extra="单位：KB，越大越清晰但耗时更长">
                    <InputNumber
                      min={20}
                      max={1024}
                      value={draft.imageMaxKb}
                      onChange={(value) => onPatch('imageMaxKb', Number(value || 100))}
                    />
                  </Form.Item>
                </Collapse.Item>
              </Collapse>
            </Form>
          </div>
        </TabPane>
      </Tabs>
    </Modal>
  );
}
