//! Renderer system
use crate::{
    mtl::{Material, MaterialDefaults},
    types::{Backend, Mesh, Texture},
};
use amethyst_assets::{AssetStorage, ProcessingState};
use amethyst_core::{
    dispatcher::{ThreadLocal, ThreadLocalObject},
    ecs::prelude::*,
    timing::Time,
    ArcThreadPool,
};
use palette::{LinSrgba, Srgba};
use rendy::{
    command::{Families, QueueId},
    factory::{Factory, ImageState},
    graph::{Graph, GraphBuilder},
    texture::palette::{load_from_linear_rgba, load_from_srgba},
};

#[cfg(feature = "profiler")]
use thread_profiler::profile_scope;

/// Auxiliary data for render graph.
pub struct InternalGraphAuxData<'a> {
    pub world: &'a World,
    pub resources: &'a Resources,
}

// FIXME: It is currently impossible to pass types with lifetimes (except for a single reference)
// to auxiliary data structures. It worked before when passing just `World`, but with legion we
// also need to pass `Resources`. To do this we have to transmute `InternalGraphAuxData<'a>` into
// `InternalGraphAuxData<'static>` and ensure that none of the graph nodes store the references.
// Simplified issue: https://github.com/rust-lang/rust/issues/51567
fn make_graph_aux_data(world: &World, resources: &Resources) -> GraphAuxData {
    unsafe { std::mem::transmute(InternalGraphAuxData { world, resources }) }
}

/// Auxiliary data for render graph. Even though it is `'static` any reference inside it must not
/// be saved in any render node. See comments on `make_graph_aux_data`.
pub type GraphAuxData = InternalGraphAuxData<'static>;

/// Graph trait implementation required by consumers. Builds a graph and manages signaling when
/// the graph needs to be rebuilt.
pub trait GraphCreator<B: Backend> {
    /// Check if graph needs to be rebuilt.
    /// This function is evaluated every frame before running the graph.
    fn rebuild(&mut self, world: &World, resources: &Resources) -> bool;

    /// Retrieve configured complete graph builder.
    fn builder(
        &mut self,
        factory: &mut Factory<B>,
        world: &World,
        resources: &Resources,
    ) -> GraphBuilder<B, GraphAuxData>;
}

/// Holds internal state of the rendering system
struct RenderState<B: Backend, G> {
    graph: Option<Graph<B, GraphAuxData>>,
    families: Families<B>,
    graph_creator: G,
}

fn rebuild_graph<B, G>(state: &mut RenderState<B, G>, world: &World, resources: &Resources)
where
    B: Backend,
    G: GraphCreator<B>,
{
    #[cfg(feature = "profiler")]
    profile_scope!("rebuild_graph");

    let mut factory = resources.get_mut::<Factory<B>>().unwrap();

    if let Some(graph) = state.graph.take() {
        #[cfg(feature = "profiler")]
        profile_scope!("dispose_graph");
        let aux = make_graph_aux_data(world, resources);
        graph.dispose(&mut *factory, &aux);
    }

    let builder = {
        #[cfg(feature = "profiler")]
        profile_scope!("run_graph_creator");
        state.graph_creator.builder(&mut factory, world, resources)
    };

    let graph = {
        #[cfg(feature = "profiler")]
        profile_scope!("build_graph");
        let aux = make_graph_aux_data(world, resources);
        builder
            .build(&mut factory, &mut state.families, &aux)
            .unwrap()
    };

    state.graph = Some(graph);
}

fn run_graph<B, G>(state: &mut RenderState<B, G>, world: &World, resources: &Resources)
where
    B: Backend,
    G: GraphCreator<B>,
{
    let mut factory = resources.get_mut::<Factory<B>>().unwrap();
    factory.maintain(&mut state.families);
    let aux = make_graph_aux_data(world, resources);
    state
        .graph
        .as_mut()
        .unwrap()
        .run(&mut factory, &mut state.families, &aux)
}

pub fn build_rendering_system<B, G>(
    world: &mut World,
    resources: &mut Resources,
    graph_creator: G,
    families: Families<B>,
) -> Box<dyn ThreadLocal>
where
    B: Backend,
    G: 'static + GraphCreator<B>,
{
    let mat = create_default_mat::<B>(resources);
    resources.insert(MaterialDefaults(mat));

    ThreadLocalObject::build(
        RenderState {
            graph: None,
            families,
            graph_creator,
        },
        |state, world, resources| {
            let rebuild = state.graph_creator.rebuild(world, resources);
            if state.graph.is_none() || rebuild {
                rebuild_graph(state, world, resources);
            }
            run_graph(state, world, resources);
        },
        move |state, world, resources| {
            let mut graph = state.graph;
            if let Some(graph) = graph.take() {
                let mut factory = resources.get_mut::<Factory<B>>().unwrap();
                log::debug!("Dispose graph");

                let aux = make_graph_aux_data(world, resources);
                graph.dispose(&mut factory, &aux);
            }

            log::debug!("Unload resources");
            if let Some(mut storage) = resources.get_mut::<AssetStorage<Mesh>>() {
                storage.unload_all();
            }
            if let Some(mut storage) = resources.get_mut::<AssetStorage<Texture>>() {
                storage.unload_all();
            }

            log::debug!("Drop families");
            drop(state.families);
        },
    )
}

/// Asset processing system for `Mesh` asset type.
pub fn build_mesh_processor<B: Backend>(
    world: &mut World,
    resources: &mut Resources,
) -> Box<dyn Schedulable> {
    resources.insert(AssetStorage::<Mesh>::default());

    SystemBuilder::<()>::new("MeshProcessorSystem")
        .write_resource::<AssetStorage<Mesh>>()
        .read_resource::<QueueId>()
        .read_resource::<Time>()
        .read_resource::<ArcThreadPool>()
        // .read_resource::<HotReloadStrategy>() // TODO: Optional resources should be OPTIONS instead.
        .read_resource::<Factory<B>>()
        .build(
            move |commands, world, (mesh_storage, queue_id, time, pool, factory), _| {
                #[cfg(feature = "profiler")]
                profile_scope!("mesh_processor");

                mesh_storage.process(
                    |b| {
                        log::trace!("Processing Mesh: {:?}", b);

                        #[cfg(feature = "profiler")]
                        profile_scope!("process_mesh");

                        b.0.build(**queue_id, &factory)
                            .map(B::wrap_mesh)
                            .map(ProcessingState::Loaded)
                            .map_err(|e| e.compat().into())
                    },
                    time.frame_number(),
                    &**pool,
                    None, // TODO: Fix strategy optional
                )
            },
        )
}

/// Asset processing system for `Mesh` asset type.
pub fn build_texture_processor<B: Backend>(
    world: &mut World,
    resources: &mut Resources,
) -> Box<dyn Schedulable> {
    resources.insert(AssetStorage::<Texture>::default());

    SystemBuilder::<()>::new("TextureProcessorSystem")
        .write_resource::<AssetStorage<Texture>>()
        .read_resource::<QueueId>()
        .read_resource::<Time>()
        .read_resource::<amethyst_core::ArcThreadPool>()
        // .read_resource::<HotReloadStrategy>() // TODO: Optional resources should be OPTIONS instead.
        .write_resource::<Factory<B>>()
        .build(
            move |commands, world, (texture_storage, queue_id, time, pool, factory), _| {
                #[cfg(feature = "profiler")]
                profile_scope!("texture_processor");

                texture_storage.process(
                    |b| {
                        log::trace!("Processing Texture: {:?}", b);

                        #[cfg(feature = "profiler")]
                        profile_scope!("process_texture");

                        b.0.build(
                            ImageState {
                                queue: **queue_id,
                                stage: rendy::hal::pso::PipelineStage::VERTEX_SHADER
                                    | rendy::hal::pso::PipelineStage::FRAGMENT_SHADER,
                                access: rendy::hal::image::Access::SHADER_READ,
                                layout: rendy::hal::image::Layout::ShaderReadOnlyOptimal,
                            },
                            &mut *factory,
                        )
                        .map(B::wrap_texture)
                        .map(ProcessingState::Loaded)
                        .map_err(|e| e.compat().into())
                    },
                    time.frame_number(),
                    &**pool,
                    None, // TODO: Fix strategy optional
                );
            },
        )
}

pub(crate) fn create_default_mat<B: Backend>(resources: &Resources) -> Material {
    use crate::mtl::TextureOffset;

    use amethyst_assets::Loader;

    let loader = resources.get::<Loader>().unwrap();

    let albedo = load_from_srgba(Srgba::new(0.5, 0.5, 0.5, 1.0));
    let emission = load_from_srgba(Srgba::new(0.0, 0.0, 0.0, 0.0));
    let normal = load_from_linear_rgba(LinSrgba::new(0.5, 0.5, 1.0, 1.0));
    let metallic_roughness = load_from_linear_rgba(LinSrgba::new(0.0, 0.5, 0.0, 0.0));
    let ambient_occlusion = load_from_linear_rgba(LinSrgba::new(1.0, 1.0, 1.0, 1.0));
    let cavity = load_from_linear_rgba(LinSrgba::new(1.0, 1.0, 1.0, 1.0));

    let tex_storage = resources.get::<AssetStorage<Texture>>().unwrap();

    let albedo = loader.load_from_data(albedo.into(), (), &tex_storage);
    let emission = loader.load_from_data(emission.into(), (), &tex_storage);
    let normal = loader.load_from_data(normal.into(), (), &tex_storage);
    let metallic_roughness = loader.load_from_data(metallic_roughness.into(), (), &tex_storage);
    let ambient_occlusion = loader.load_from_data(ambient_occlusion.into(), (), &tex_storage);
    let cavity = loader.load_from_data(cavity.into(), (), &tex_storage);

    Material {
        alpha_cutoff: 0.01,
        albedo,
        emission,
        normal,
        metallic_roughness,
        ambient_occlusion,
        cavity,
        uv_offset: TextureOffset::default(),
    }
}
