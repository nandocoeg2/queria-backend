import React, { useRef, useEffect, useState, useMemo } from 'react';
import { Canvas, useFrame, useThree } from '@react-three/fiber';
import { OrbitControls } from '@react-three/drei';
import { EffectComposer, Bloom } from '@react-three/postprocessing';
import * as THREE from 'three';
import NodeCloud from './NodeCloud';
import type { GraphNode } from './NodeCloud';
import EdgeLines from './EdgeLines';
import type { GraphEdge } from './EdgeLines';

// Camera controller that handles smooth fly-to lerp animation
function CameraController({ selectedNode }: { selectedNode: GraphNode | null }) {
  const { camera } = useThree();
  const controlsRef = useRef<any>(null);

  useFrame(() => {
    if (selectedNode) {
      // Calculate target camera position (offset slightly on Z)
      const targetPos = new THREE.Vector3(selectedNode.x, selectedNode.y, selectedNode.z + 4.0);
      camera.position.lerp(targetPos, 0.06);

      // Lerp orbit controls target to look exactly at the node
      const targetLookAt = new THREE.Vector3(selectedNode.x, selectedNode.y, selectedNode.z);
      if (controlsRef.current) {
        controlsRef.current.target.lerp(targetLookAt, 0.06);
        controlsRef.current.update();
      }
    }
  });

  return (
    <OrbitControls
      ref={controlsRef}
      enableZoom={true}
      enablePan={true}
      maxDistance={40}
      minDistance={1}
      dampingFactor={0.05}
      enableDamping={true}
    />
  );
}

export default function ThreeCanvas() {
  const [projects, setProjects] = useState<any[]>([]);
  const [selectedProject, setSelectedProject] = useState<string>('');
  const [nodes, setNodes] = useState<GraphNode[]>([]);
  const [edges, setEdges] = useState<GraphEdge[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');

  // Interaction states
  const [hoveredId, setHoveredId] = useState<string | null>(null);
  const [hoveredNode, setHoveredNode] = useState<GraphNode | null>(null);
  const [hoveredPos, setHoveredPos] = useState<{ x: number; y: number } | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [selectedNode, setSelectedNode] = useState<GraphNode | null>(null);

  // Map representation of nodes for quick edge matching
  const nodesMap = useMemo(() => {
    const map = new Map<string, GraphNode>();
    nodes.forEach((node) => map.set(node.id, node));
    return map;
  }, [nodes]);

  // Fetch projects list on mount
  useEffect(() => {
    async function loadProjects() {
      try {
        const response = await fetch('/api/v1/projects');
        if (response.ok) {
          const data = await response.json();
          setProjects(data);
          if (data.length > 0) {
            setSelectedProject(data[0].slug);
          }
        } else {
          setError('Failed to fetch projects');
        }
      } catch (err) {
        console.error('Error fetching projects:', err);
        setError('Connection failed');
      }
    }
    loadProjects();
  }, []);

  // Fetch sources and knowledge items when selectedProject changes
  useEffect(() => {
    if (!selectedProject) return;

    async function loadGraphData() {
      setLoading(true);
      setError('');
      try {
        // Fetch source documents
        const docsRes = await fetch(`/api/v1/sources?project_slug=${selectedProject}`);
        // Fetch first page of knowledge items (limit 100 for graph visualizer rendering scale)
        const itemsRes = await fetch(`/api/v1/knowledge-items?project_slug=${selectedProject}&limit=100`);

        if (docsRes.ok && itemsRes.ok) {
          const docsData = await docsRes.json();
          const itemsData = await itemsRes.json();

          // Calculate 3D Layout
          const newNodes: GraphNode[] = [];
          const newEdges: GraphEdge[] = [];

          const docCoords = new Map<string, { x: number; y: number; z: number }>();

          // 1. Distribute Document Nodes in a sphere of radius 15
          const numDocs = docsData.length;
          docsData.forEach((doc: any, idx: number) => {
            // Golden spiral on sphere distribution
            let x = 0, y = 0, z = 0;
            if (numDocs > 1) {
              y = 1.0 - (idx / (numDocs - 1)) * 2.0;
              const radius = Math.sqrt(1.0 - y * y);
              const theta = 2.399963 * idx; // Golden angle in radians
              x = Math.cos(theta) * radius;
              z = Math.sin(theta) * radius;
            } else {
              x = 0; y = 0; z = 0;
            }

            const scale = 14.0; // scale distance from center
            const coords = { x: x * scale, y: y * scale, z: z * scale };
            docCoords.set(doc.id, coords);

            newNodes.push({
              id: doc.id,
              type: 'document',
              name: doc.relative_path,
              x: coords.x,
              y: coords.y,
              z: coords.z,
              color: '#b65c21', // Sahara Gold/Sand for documents
              size: 0.35,
            });
          });

          // Group knowledge items by their parent document ID
          const itemsByDoc = new Map<string, any[]>();
          itemsData.items.forEach((item: any) => {
            const docId = item.source_document_id;
            if (!docId) return;
            if (!itemsByDoc.has(docId)) {
              itemsByDoc.set(docId, []);
            }
            itemsByDoc.get(docId)!.push(item);
          });

          // 2. Distribute Knowledge Item Nodes in a shell of radius 2 around their parent Document
          itemsByDoc.forEach((items, docId) => {
            const parentCoord = docCoords.get(docId);
            if (!parentCoord) return;

            const numItems = items.length;
            items.forEach((item: any, idx: number) => {
              // spherical distribution around parent
              let dx = 0, dy = 0, dz = 0;
              if (numItems > 0) {
                const theta = (idx * 2.4) % (2.0 * Math.PI);
                const phi = Math.acos(1.0 - 2.0 * ((idx + 0.5) / numItems));
                const dist = 1.8 + Math.random() * 0.8; // radius between 1.8 and 2.6
                dx = dist * Math.sin(phi) * Math.cos(theta);
                dy = dist * Math.sin(phi) * Math.sin(theta);
                dz = dist * Math.cos(phi);
              }

              const itemX = parentCoord.x + dx;
              const itemY = parentCoord.y + dy;
              const itemZ = parentCoord.z + dz;

              // Color based on status
              let color = '#964407'; // Approved
              if (item.status === 'draft') color = '#9ca3af'; // Grey for draft
              if (item.status === 'proposed') color = '#eab308'; // Amber/Yellow
              if (item.status === 'rejected') color = '#ef4444'; // Red

              newNodes.push({
                id: item.id,
                type: 'knowledge_item',
                name: item.title,
                x: itemX,
                y: itemY,
                z: itemZ,
                color,
                size: 0.16,
              });

              // Hierarchical edge (Document -> Knowledge Item)
              newEdges.push({
                source: docId,
                target: item.id,
                type: 'has_knowledge',
                color: '#dbc1b5', // subtle outlining color
              });
            });
          });

          // 3. Add semantic cross-document edges based on shared tags
          let crossEdgesCount = 0;
          for (let i = 0; i < itemsData.items.length; i++) {
            if (crossEdgesCount > 50) break;
            const itemA = itemsData.items[i];
            const tagsA = itemA.tags || [];
            if (tagsA.length === 0) continue;

            for (let j = i + 1; j < itemsData.items.length; j++) {
              const itemB = itemsData.items[j];
              const tagsB = itemB.tags || [];
              
              // Find intersection
              const common = tagsA.filter((t: string) => tagsB.includes(t));
              if (common.length > 0 && itemA.source_document_id !== itemB.source_document_id) {
                newEdges.push({
                  source: itemA.id,
                  target: itemB.id,
                  type: 'semantic_relation',
                  color: '#944242', // Warm tertiary/red color for semantic links
                });
                crossEdgesCount++;
                if (crossEdgesCount > 50) break;
              }
            }
          }

          setNodes(newNodes);
          setEdges(newEdges);
          setSelectedId(null);
          setSelectedNode(null);
        } else {
          setError('Failed to fetch graph details');
        }
      } catch (err) {
        console.error('Error fetching graph data:', err);
        setError('Error loading knowledge graph');
      } finally {
        setLoading(false);
      }
    }
    loadGraphData();
  }, [selectedProject]);

  const handleHover = (id: string | null, e?: any) => {
    setHoveredId(id);
    if (id) {
      const node = nodes.find((n) => n.id === id);
      setHoveredNode(node || null);
      if (e) {
        // Calculate canvas/screen coordinates for the tooltip
        setHoveredPos({
          x: e.clientX,
          y: e.clientY - 40,
        });
      }
    } else {
      setHoveredNode(null);
      setHoveredPos(null);
    }
  };

  const handleClick = (id: string) => {
    setSelectedId(id);
    const node = nodes.find((n) => n.id === id);
    setSelectedNode(node || null);
  };

  // Fetch full details of selected knowledge item from API
  const [selectedItemDetail, setSelectedItemDetail] = useState<any>(null);
  useEffect(() => {
    if (!selectedId || !selectedNode || selectedNode.type !== 'knowledge_item') {
      setSelectedItemDetail(null);
      return;
    }
    async function loadDetail() {
      try {
        const res = await fetch(`/api/v1/knowledge-items/${selectedId}`);
        if (res.ok) {
          const data = await res.json();
          setSelectedItemDetail(data);
        }
      } catch (err) {
        console.error('Failed to load item detail:', err);
      }
    }
    loadDetail();
  }, [selectedId, selectedNode]);

  return (
    <div className="flex flex-col gap-4 w-full">
      {/* Selector Bar & Legend */}
      <div className="flex flex-wrap justify-between items-center bg-[#fff8f5] border border-[#dbc1b5]/60 rounded-xl p-4 gap-4">
        <div className="flex items-center gap-2">
          <label htmlFor="project_select" className="text-xs font-bold uppercase tracking-wider text-[#554339]">Active Project:</label>
          <select
            id="project_select"
            value={selectedProject}
            onChange={(e) => setSelectedProject(e.target.value)}
            className="px-3 py-1.5 border border-[#dbc1b5] rounded-md bg-[#ffffff] text-sm text-[#221a14] outline-none focus:border-[#964407]"
          >
            {projects.map((proj) => (
              <option key={proj.slug} value={proj.slug}>
                {proj.name}
              </option>
            ))}
          </select>
        </div>

        {/* Legend */}
        <div className="flex flex-wrap gap-4 text-xs font-body text-[#554339]">
          <div className="flex items-center gap-1.5">
            <span className="w-3 h-3 rounded-full bg-[#b65c21]"></span>
            <span>Source Document</span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="w-3 h-3 rounded-full bg-[#964407]"></span>
            <span>Approved Knowledge</span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="w-3 h-3 rounded-full bg-[#eab308]"></span>
            <span>Proposed</span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="w-3 h-3 rounded-full bg-[#9ca3af]"></span>
            <span>Draft</span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="w-4 h-0.5 inline-block bg-[#944242]"></span>
            <span>Semantic Link</span>
          </div>
        </div>
      </div>

      {/* Main 3D Canvas Box */}
      <div className="relative w-full h-[550px] bg-[#221a14] rounded-2xl overflow-hidden border border-[#887368]/30 shadow-2xl">
        {loading && (
          <div className="absolute inset-0 bg-[#221a14]/80 flex flex-col justify-center items-center z-20 gap-3">
            <div className="w-10 h-10 border-4 border-[#964407] border-t-transparent rounded-full animate-spin"></div>
            <p className="text-sm font-body text-[#dbc1b5]">Calculating knowledge layout...</p>
          </div>
        )}

        {error && (
          <div className="absolute inset-0 bg-[#221a14]/90 flex justify-center items-center z-20 p-4">
            <p className="text-sm text-[#ef4444] font-medium">{error}</p>
          </div>
        )}

        {/* Floating Tooltip */}
        {hoveredNode && hoveredPos && (
          <div
            style={{
              position: 'fixed',
              left: hoveredPos.x + 12,
              top: hoveredPos.y,
              pointerEvents: 'none',
              zIndex: 100,
            }}
            className="px-3 py-1.5 bg-[#fff8f5] text-[#221a14] border border-[#dbc1b5] rounded shadow-lg text-xs font-body max-w-xs"
          >
            <span className="font-bold text-[10px] uppercase text-[#964407]">
              {hoveredNode.type === 'document' ? 'File' : 'Knowledge Item'}
            </span>
            <p className="truncate font-semibold">{hoveredNode.name}</p>
          </div>
        )}

        <Canvas camera={{ position: [0, 0, 24], fov: 50 }} gl={{ antialias: true }}>
          <ambientLight intensity={0.2} />
          <directionalLight position={[10, 15, 10]} intensity={1.0} color="#fff1ea" />
          <pointLight position={[-10, -10, -10]} intensity={0.5} />
          
          <NodeCloud
            nodes={nodes}
            hoveredId={hoveredId}
            selectedId={selectedId}
            onHover={handleHover}
            onClick={handleClick}
          />

          <EdgeLines
            edges={edges}
            nodesMap={nodesMap}
            hoveredId={hoveredId}
            selectedId={selectedId}
          />

          <CameraController selectedNode={selectedNode} />

          <EffectComposer>
            <Bloom
              intensity={1.5}
              luminanceThreshold={0.2}
              luminanceSmoothing={0.9}
              mipmapBlur
            />
          </EffectComposer>
        </Canvas>
      </div>

      {/* Selected Node Details Panel */}
      {selectedNode && (
        <div className="bg-[#fff8f5] border border-[#dbc1b5] rounded-2xl p-6 shadow-md animate-in fade-in slide-in-from-bottom-2 duration-300">
          <div className="flex justify-between items-start border-b border-[#dbc1b5]/50 pb-4 mb-4">
            <div>
              <span className="text-[10px] font-bold uppercase tracking-wider bg-[#b65c21] text-[#ffffff] px-2 py-0.5 rounded-full">
                {selectedNode.type === 'document' ? 'Source File' : 'Knowledge Item'}
              </span>
              <h2 className="text-xl font-headline font-semibold text-[#221a14] mt-2">
                {selectedNode.name}
              </h2>
            </div>
            <button
              onClick={() => {
                setSelectedId(null);
                setSelectedNode(null);
              }}
              className="text-xs font-semibold text-[#605e59] hover:text-[#964407] cursor-pointer"
            >
              Close Details ✕
            </button>
          </div>

          {/* Details Content */}
          <div className="font-body text-[#221a14] text-sm">
            {selectedNode.type === 'document' ? (
              <div className="space-y-2">
                <p>
                  <strong>Document Path:</strong> <code>{selectedNode.name}</code>
                </p>
                <div className="pt-2">
                  <a
                    href={`/admin/sources?project_slug=${selectedProject}`}
                    className="text-xs font-bold text-[#964407] hover:underline"
                  >
                    View Document Sources →
                  </a>
                </div>
              </div>
            ) : selectedItemDetail ? (
              <div className="space-y-4">
                <div className="grid grid-cols-2 sm:grid-cols-4 gap-4 text-xs">
                  <div>
                    <span className="block text-gray-500 uppercase tracking-wider">Category</span>
                    <strong className="text-gray-800">{selectedItemDetail.category}</strong>
                  </div>
                  <div>
                    <span className="block text-gray-500 uppercase tracking-wider">Scope</span>
                    <strong className="text-gray-800 uppercase">{selectedItemDetail.scope}</strong>
                  </div>
                  <div>
                    <span className="block text-gray-500 uppercase tracking-wider">Status</span>
                    <span className={`inline-block font-semibold uppercase ${
                      selectedItemDetail.status === 'approved' ? 'text-green-700' : 'text-amber-700'
                    }`}>
                      {selectedItemDetail.status}
                    </span>
                  </div>
                  {selectedItemDetail.approved_at && (
                    <div>
                      <span className="block text-gray-500 uppercase tracking-wider">Approved At</span>
                      <strong className="text-gray-800">
                        {new Date(selectedItemDetail.approved_at).toLocaleDateString()}
                      </strong>
                    </div>
                  )}
                </div>

                {selectedItemDetail.tags && selectedItemDetail.tags.length > 0 && (
                  <div className="flex flex-wrap gap-1.5 pt-2">
                    {selectedItemDetail.tags.map((t: string) => (
                      <span key={t} className="text-[10px] font-semibold bg-[#e6e2db] text-[#554339] px-2 py-0.5 rounded">
                        #{t}
                      </span>
                    ))}
                  </div>
                )}

                <div className="mt-4 pt-4 border-t border-[#dbc1b5]/30">
                  <span className="block text-xs text-gray-500 uppercase tracking-wider mb-2">Knowledge Content</span>
                  <div className="bg-[#ffffff] border border-[#dbc1b5]/40 rounded-lg p-4 font-mono text-xs overflow-auto max-h-[300px] whitespace-pre-wrap leading-relaxed">
                    {selectedItemDetail.body}
                  </div>
                </div>
              </div>
            ) : (
              <div className="flex justify-center p-4">
                <div className="w-5 h-5 border-2 border-[#964407] border-t-transparent rounded-full animate-spin"></div>
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
