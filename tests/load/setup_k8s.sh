#!/bin/bash
set -eu

# Declare variables
GCLOUD=$(which gcloud)
SED=$(which sed)
KUBECTL=$(which kubectl)
GOOGLE_CLOUD_PROJECT=$(gcloud config get-value project)
CLUSTER='autopush-locust-load-test'
SCOPE='https://www.googleapis.com/auth/cloud-platform'
REGION='us-central1'
WORKER_COUNT=5
MACHINE_TYPE='n1-standard-2'
BOLD=$(tput bold)
NORM=$(tput sgr0)
DIRECTORY=$(pwd)

AUTOPUSH_DIRECTORY=$DIRECTORY/kubernetes-config
MASTER_FILE=locust-master-controller.yml
WORKER_FILE=locust-worker-controller.yml
SERVICE_FILE=locust-master-service.yml

LOCUST_IMAGE_TAG=$(git log -1 --pretty=format:%h)
echo "Image tag for locust is set to: ${LOCUST_IMAGE_TAG}"

##Declare variables to be replaced later in the YAML file using the sed commands
LOCUST_CSV='autopush'
LOCUST_USERS='20'
LOCUST_SPAWN_RATE='1'
LOCUST_RUN_TIME='600' # 10 minutes
LOCUST_LOGLEVEL='INFO'
SERVER_URL=${SERVER_URL}
ENDPOINT_URL=${ENDPOINT_URL}

SetupGksCluster()
{

    #Configure Kubernetes
    echo -e "==================== Prepare environments with set of environment variables "
    echo -e "==================== Set Kubernetes Cluster "
    export CLUSTER=$CLUSTER
    echo -e "==================== Set SCOPE "
    export SCOPE=$SCOPE

    echo -e "==================== Refresh Kubeconfig at path ~/.kube/config "
    $GCLOUD container clusters get-credentials $CLUSTER --region $REGION --project $GOOGLE_CLOUD_PROJECT

    ##Build Docker Images
    echo -e "==================== Build the Docker image and store it in your project's container registry. Tag with the latest commit hash "
    $GCLOUD builds submit --tag gcr.io/$GOOGLE_CLOUD_PROJECT/locust-autopush:$LOCUST_IMAGE_TAG
    echo -e "==================== Verify that the Docker image is in your project's container repository"
    $GCLOUD container images list | grep locust-autopush

    ##Deploying the Locust master and worker nodes
    echo -e "==================== Update Kubernetes Manifests "
    echo -e "==================== Replace the target host, project ID and environment variables in the locust-master-controller.yml and locust-worker-controller.yml files"

    FILES=($MASTER_FILE $WORKER_FILE)
    for file in "${FILES[@]}"
    do

      # LOCUST_CSV
        $SED -i -e "s|\[PROJECT_ID\]|$GOOGLE_CLOUD_PROJECT|g" $AUTOPUSH_DIRECTORY/$file
        $SED -i -e "s|\[WORKER_COUNT\]|$WORKER_COUNT|g" $AUTOPUSH_DIRECTORY/$file
        $SED -i -e "s|\[LOCUST_IMAGE_TAG\]|$LOCUST_IMAGE_TAG|g" $AUTOPUSH_DIRECTORY/$file
        $SED -i -e "s|\[LOCUST_CSV\]|$LOCUST_CSV|g" $AUTOPUSH_DIRECTORY/$file
        $SED -i -e "s|\[LOCUST_USERS\]|$LOCUST_USERS|g" $AUTOPUSH_DIRECTORY/$file
        $SED -i -e "s|\[LOCUST_SPAWN_RATE\]|$LOCUST_SPAWN_RATE|g" $AUTOPUSH_DIRECTORY/$file
        $SED -i -e "s|\[LOCUST_RUN_TIME\]|$LOCUST_RUN_TIME|g" $AUTOPUSH_DIRECTORY/$file
        $SED -i -e "s|\[LOCUST_LOGLEVEL\]|$LOCUST_LOGLEVEL|g" $AUTOPUSH_DIRECTORY/$file
        $SED -i -e "s|\[SERVER_URL\]|$SERVER_URL|g" $AUTOPUSH_DIRECTORY/$file
        $SED -i -e "s|\[ENDPOINT_URL\]|$ENDPOINT_URL|g" $AUTOPUSH_DIRECTORY/$file

    done

    ##Deploy the Locust master and worker nodes using Kubernetes Manifests
    echo -e "==================== Deploy the Locust master and worker nodes"
    $KUBECTL apply -f $AUTOPUSH_DIRECTORY/$MASTER_FILE
    $KUBECTL apply -f $AUTOPUSH_DIRECTORY/$SERVICE_FILE
    $KUBECTL apply -f $AUTOPUSH_DIRECTORY/$WORKER_FILE

    echo -e "==================== Verify the Locust deployments & Services"
    $KUBECTL get pods -o wide
    $KUBECTL get services
}

echo "==================== The script is used to create & delete the GKE cluster"
echo "==================== Do you want to create or setup the existing or delete GKE cluster? Select ${BOLD}create or delete or setup ${NORM}"
while :
do
    read response
    case $response in
        create) #Setup Kubernetes Cluster
            echo -e "==================== Creating the GKE cluster "
            $GCLOUD container clusters create $CLUSTER --region $REGION --scopes $SCOPE --enable-autoscaling --min-nodes "5" --max-nodes "10" --scopes=logging-write,storage-ro --addons HorizontalPodAutoscaling,HttpLoadBalancing  --machine-type $MACHINE_TYPE
            SetupGksCluster
            break
            ;;
        delete)
            echo -e "==================== Delete the GKE cluster "
            $GCLOUD container clusters delete $CLUSTER --region $REGION
            break
            ;;
        setup)
            echo -e "==================== Setup the GKE cluster "
            SetupGksCluster
            break
            ;;
        *)
            echo -e "==================== Incorrect input! "
            break
            ;;
    esac
done